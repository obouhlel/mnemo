//! Ruban d'icônes et barres latérales : explorateur de fichiers (arborescence), recherche, tags
//! à gauche ; rétroliens, panneaux des plugins et tickets liés à droite.

use super::*;

/// Marge horizontale commune aux panneaux latéraux (en-têtes et contenu alignés dessus).
pub(super) const SIDE_PAD: f32 = 8.0;
/// Hauteur des en-têtes (onglets, panneaux) ; les icônes y sont centrées à `HEADER_H - 14` du haut.
pub(super) const HEADER_H: f32 = 40.0;

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

/// État partagé pendant le dessin de l'arborescence.
struct TreeCx<'a> {
    open: Option<Uuid>,
    actions: &'a mut Vec<UiAction>,
    expanded: &'a std::collections::HashSet<String>,
    sort: SortOrder,
    icons: &'a HashMap<String, String>,
    /// un élément (de l'explorateur ou du système) est en train d'être glissé
    dragging: bool,
    /// dossier survolé pendant le glissement ("" = racine)
    target: Option<String>,
    accent: Color32,
}

/// Rend une ligne déplaçable et note le dossier visé si le pointeur la survole pendant un glissement.
fn dnd_row(cx: &mut TreeCx, r: egui::Response, item: DragItem, target: &str) -> egui::Response {
    let r = r.interact(egui::Sense::drag());
    r.dnd_set_drag_payload(item);
    if cx.dragging && r.contains_pointer() {
        cx.target = Some(target.to_string());
    }
    r
}

fn tree_ui(ui: &mut egui::Ui, folder: &TreeFolder, depth: usize, cx: &mut TreeCx) {
    let mut folders: Vec<&TreeFolder> = folder.folders.values().collect();
    if cx.sort == SortOrder::NameDesc {
        folders.reverse();
    }
    for sub in folders {
        let is_open = cx.expanded.contains(&sub.path.to_lowercase());
        let top = ui.cursor().top();
        let icon = cx.icons.get(&sub.path.to_lowercase()).map(String::as_str);
        let r = theme::tree_row_icon(ui, &sub.name, depth, false, Some(is_open), icon);
        let r = dnd_row(cx, r, DragItem::Folder(sub.path.clone()), &sub.path);
        if r.clicked() {
            cx.actions.push(UiAction::ToggleFolder(sub.path.clone()));
        }
        r.context_menu(|ui| {
            if ui.button("Nouvelle note").clicked() {
                cx.actions.push(UiAction::NewNoteIn(sub.path.clone()));
                ui.close();
            }
            if ui.button("Nouveau dossier").clicked() {
                cx.actions.push(UiAction::NewFolderIn(sub.path.clone()));
                ui.close();
            }
            if ui.button("Nouveau whiteboard").clicked() {
                cx.actions.push(UiAction::NewWhiteboardIn(sub.path.clone()));
                ui.close();
            }
            if ui.button("Renommer / déplacer…").clicked() {
                cx.actions.push(UiAction::RenameFolder(sub.path.clone()));
                ui.close();
            }
            if ui.button("Changer l'icône…").clicked() {
                cx.actions.push(UiAction::PickIcon(sub.path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                cx.actions.push(UiAction::DeleteFolder(sub.path.clone()));
                ui.close();
            }
        });
        if is_open {
            tree_ui(ui, sub, depth + 1, cx);
        }
        // dossier visé : on surligne tout son bloc (ligne + contenu déplié)
        if cx.target.as_deref() == Some(sub.path.as_str()) {
            let block =
                egui::Rect::from_x_y_ranges(r.rect.x_range(), top..=ui.cursor().top()).shrink2(egui::vec2(2.0, 0.0));
            ui.painter().rect(
                block,
                5.0,
                cx.accent.gamma_multiply(0.10),
                egui::Stroke::new(1.0, cx.accent.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        }
    }
    let mut notes: Vec<&(String, Uuid, chrono::DateTime<chrono::Utc>)> = folder.notes.iter().collect();
    match cx.sort {
        SortOrder::NameAsc => {}
        SortOrder::NameDesc => notes.reverse(),
        SortOrder::Modified => notes.sort_by_key(|n| std::cmp::Reverse(n.2)),
    }
    for (name, id, _) in notes {
        let path = if folder.path.is_empty() { name.clone() } else { format!("{}/{}", folder.path, name) };
        let icon = cx.icons.get(&path.to_lowercase()).map(String::as_str);
        let r = theme::tree_row_icon(ui, name, depth, cx.open == Some(*id), None, icon);
        // déposer sur une note = déposer dans son dossier
        let r = dnd_row(cx, r, DragItem::Note(*id, path.clone()), &folder.path);
        if r.clicked() {
            if ui.input(|i| i.modifiers.command) {
                cx.actions.push(UiAction::OpenNewTab(*id));
            } else {
                cx.actions.push(UiAction::OpenId(*id));
            }
        }
        if r.middle_clicked() {
            cx.actions.push(UiAction::OpenNewTab(*id));
        }
        r.context_menu(|ui| {
            if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                cx.actions.push(UiAction::OpenNewTab(*id));
                ui.close();
            }
            if ui.button("Renommer / déplacer…").clicked() {
                cx.actions.push(UiAction::RenameNote(*id, path.clone()));
                ui.close();
            }
            if ui.button("Copier le lien [[…]]").clicked() {
                ui.ctx().copy_text(format!("[[{name}]]"));
                ui.close();
            }
            if ui.button("Changer l'icône…").clicked() {
                cx.actions.push(UiAction::PickIcon(path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                cx.actions.push(UiAction::DeleteNote(*id, path.clone()));
                ui.close();
            }
        });
    }
    // pièces jointes (images, whiteboards, PDF…), après les notes, avec leur extension
    for rel in &folder.files {
        let name = basename(rel);
        let icon = cx.icons.get(&rel.to_lowercase()).map(String::as_str).unwrap_or(super::attachments::file_icon(rel));
        let r = theme::tree_row_icon(ui, name, depth, false, None, Some(icon));
        if r.clicked() {
            cx.actions.push(UiAction::OpenFile(rel.clone()));
        }
        r.context_menu(|ui| {
            if ui.button("Ouvrir").clicked() {
                cx.actions.push(UiAction::OpenFile(rel.clone()));
                ui.close();
            }
            if ui.button("Intégrer dans la note ouverte").clicked() {
                cx.actions.push(UiAction::InsertEmbed(rel.clone()));
                ui.close();
            }
            if ui.button("Copier l'intégration ![[…]]").clicked() {
                ui.ctx().copy_text(format!("![[{name}]]"));
                ui.close();
            }
            ui.separator();
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                cx.actions.push(UiAction::DeleteFile(rel.clone()));
                ui.close();
            }
        });
    }
}

impl MnemoApp {
    pub(super) fn ribbon(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            // bouton de 30 px centré à 26 px du haut, comme le titre des onglets et les icônes des panneaux
            ui.add_space(HEADER_H - 14.0 - 15.0);
            if theme::icon_button(ui, icon::PANEL_LEFT, "Afficher / masquer le panneau latéral", false).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::ToggleLeft));
            }
            ui.add_space(10.0);
            // seulement des fonctions intégrées : les commandes des plugins sont dans la palette (Ctrl+P)
            let items: [(&str, &str, CoreCmd); 5] = [
                (icon::DASHBOARD, "Tableau de bord : projets et tickets (Ctrl+Shift+D)", CoreCmd::Dashboard),
                (icon::SWITCHER, "Ouvrir le sélecteur rapide", CoreCmd::Switcher),
                (icon::GRAPH, "Ouvrir la vue graphique", CoreCmd::Graph),
                (icon::WHITEBOARD, "Nouveau whiteboard (dessin libre)", CoreCmd::NewWhiteboard),
                (icon::PALETTE, "Ouvrir la palette de commandes", CoreCmd::Palette),
            ];
            let tab = &self.tabs[self.active_tab];
            let (dash, graph) = (tab.dashboard, tab.graph);
            for (ic, tip, c) in items {
                // le bouton de la vue affichée reste en surbrillance
                let active = matches!(c, CoreCmd::Dashboard) && dash || matches!(c, CoreCmd::Graph) && graph;
                if theme::icon_button(ui, ic, tip, active).clicked() {
                    self.actions.push(UiAction::Core(c));
                }
                ui.add_space(2.0);
            }
        });
        // bas du ruban : aide et paramètres
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Center), |ui| {
            ui.add_space(5.0); // même marge qu'en haut du ruban
            if theme::icon_button(ui, icon::SETTINGS, "Paramètres (Ctrl+,)", self.show_settings).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::Settings));
            }
            ui.add_space(2.0);
            if theme::icon_button(ui, icon::HELP, "Aide et raccourcis", false).clicked() {
                self.settings_tab = SettingsTab::About;
                self.show_settings = true;
            }
            ui.add_space(2.0);
            self.sync_indicator(ui);
        });
    }

    pub(super) fn left_sidebar(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.palette();
        // header: view tabs
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), HEADER_H),
            // en bas de la rangée, alignées sur le titre des onglets
            egui::Layout::left_to_right(egui::Align::Max),
            |ui| {
                ui.add_space(SIDE_PAD);
                ui.spacing_mut().item_spacing.x = 4.0;
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
        let body_h = ui.available_height().max(50.0);
        ui.allocate_ui(egui::vec2(ui.available_width(), body_h), |ui| {
            ui.set_min_height(body_h);
            match self.left_tab {
                LeftTab::Files => {
                    // même hauteur que l'en-tête de la note (flèches, fil d'Ariane) pour aligner les rangées
                    let row = egui::vec2(ui.available_width(), 40.0);
                    ui.allocate_ui_with_layout(row, egui::Layout::left_to_right(egui::Align::Center), |ui| {
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
                    let ctx = ui.ctx().clone();
                    let internal = egui::DragAndDrop::has_payload_of_type::<DragItem>(&ctx);
                    let external = ctx.input(|i| !i.raw.hovered_files.is_empty());
                    let mut cx = TreeCx {
                        open: self.open.as_ref().map(|n| n.id),
                        actions: &mut self.actions,
                        expanded: &self.expanded,
                        sort: self.sort,
                        icons: &self.icons,
                        dragging: internal || external,
                        target: None,
                        accent: p.accent,
                    };
                    let scroll = egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(SIDE_PAD as i8 - 2, 0)).show(
                            ui,
                            |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                tree_ui(ui, &self.tree, 0, &mut cx);
                                let rest = ui.allocate_response(
                                    ui.available_size().max(egui::vec2(10.0, 40.0)),
                                    egui::Sense::click(),
                                );
                                if cx.dragging && rest.contains_pointer() {
                                    cx.target = Some(String::new());
                                }
                                rest.context_menu(|ui| {
                                    if ui.button("Nouvelle note").clicked() {
                                        cx.actions.push(UiAction::Core(CoreCmd::NewNote));
                                        ui.close();
                                    }
                                    if ui.button("Nouveau dossier").clicked() {
                                        cx.actions.push(UiAction::Core(CoreCmd::NewFolder));
                                        ui.close();
                                    }
                                });
                            },
                        );
                    });
                    let target = cx.target.take();
                    // racine visée : contour de toute la zone
                    if target.as_deref() == Some("") {
                        ui.painter().rect_stroke(
                            scroll.inner_rect.shrink(3.0),
                            5.0,
                            egui::Stroke::new(1.0, p.accent.gamma_multiply(0.6)),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if internal {
                        // étiquette qui suit le pointeur
                        if let (Some(item), Some(pos)) =
                            (egui::DragAndDrop::payload::<DragItem>(&ctx), ctx.pointer_interact_pos())
                        {
                            egui::Area::new(egui::Id::unique("tree-drag-ghost"))
                                .fixed_pos(pos + egui::vec2(14.0, 8.0))
                                .order(egui::Order::Tooltip)
                                .interactable(false)
                                .show(&ctx, |ui| {
                                    egui::Frame::popup(ui.style()).inner_margin(egui::Margin::symmetric(8, 4)).show(
                                        ui,
                                        |ui| {
                                            ui.label(RichText::new(basename(item.path())).color(p.text));
                                        },
                                    );
                                });
                        }
                        if ctx.input(|i| i.pointer.any_released())
                            && let Some(t) = &target
                            && let Some(item) = egui::DragAndDrop::take_payload::<DragItem>(&ctx)
                        {
                            self.actions.push(UiAction::Move((*item).clone(), t.clone()));
                        }
                    }
                    // fichiers du système déposés sur l'explorateur
                    if external {
                        self.os_drop_target = Some((target.clone().unwrap_or_default(), ctx.input(|i| i.time)));
                    }
                }
                LeftTab::Search => {
                    ui.add_space(6.0);
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(SIDE_PAD as i8, 0)).show(ui, |ui| {
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
                    ui.add_space(8.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(SIDE_PAD as i8 - 2, 0)).show(
                            ui,
                            |ui| {
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
                            },
                        );
                    });
                }
                LeftTab::Tags => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(SIDE_PAD as i8 - 2, 6)).show(
                            ui,
                            |ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                if self.tags.is_empty() {
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new("Aucun tag. Écrivez #montag dans une note.").color(p.faint),
                                        )
                                        .wrap(),
                                    );
                                }
                                for (tag, count) in &self.tags {
                                    if theme::count_row(ui, &format!("#{tag}"), &count.to_string()).clicked() {
                                        self.actions.push(UiAction::OpenTag(tag.clone()));
                                    }
                                }
                            },
                        );
                    });
                }
            }
        });
    }

    pub(super) fn right_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
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
            egui::vec2(ui.available_width(), HEADER_H),
            // en bas de la rangée, alignées sur le titre des onglets
            egui::Layout::left_to_right(egui::Align::Max),
            |ui| {
                ui.add_space(SIDE_PAD);
                ui.spacing_mut().item_spacing.x = 4.0;
                for (i, (ic, name)) in tabs.iter().enumerate() {
                    if theme::icon_button_sized(ui, ic, name, self.right_tab == i, true, 16.0).clicked() {
                        self.right_tab = i;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
                    ui.add_space(SIDE_PAD);
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
            egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 8, bottom: 4 }).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.open.is_none() {
                    ui.horizontal(|ui| ui.label(RichText::new("Aucun fichier actif.").color(p.faint)));
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

    /// Déplace une note ou un dossier dans `target` ("" = racine), liens [[…]] mis à jour par le worker.
    pub(super) fn move_item(&mut self, item: DragItem, target: &str, now: f64) {
        let from = item.path().to_string();
        if parent(&from).unwrap_or("") == target {
            return; // déjà là
        }
        let to = if target.is_empty() { basename(&from).to_string() } else { format!("{target}/{}", basename(&from)) };
        self.flush_save();
        match item {
            DragItem::Note(id, _) => self.db.send(Cmd::Rename { id, new_path: to }),
            DragItem::Folder(_) => {
                let (f, t) = (from.to_lowercase(), target.to_lowercase());
                if t == f || t.starts_with(&format!("{f}/")) {
                    self.notice("Impossible de déplacer un dossier dans lui-même", true, now);
                    return;
                }
                if self.expanded.remove(&f) {
                    self.expanded.insert(to.to_lowercase());
                }
                self.db.send(Cmd::RenameFolder { from, to });
            }
        }
        if !target.is_empty() {
            self.expanded.insert(target.to_lowercase());
        }
        self.need.notes = true;
        self.need.folders = true;
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
        for f in &self.files {
            let target = match parent(f) {
                Some(p) => folder_mut(&mut root, p),
                None => &mut root,
            };
            target.files.push(f.clone());
        }
        fn sort(f: &mut TreeFolder) {
            f.notes.sort_by_key(|n| n.0.to_lowercase());
            f.files.sort_by_key(|n| n.to_lowercase());
            f.folders.values_mut().for_each(sort);
        }
        sort(&mut root);
        self.tree = root;
    }
}
