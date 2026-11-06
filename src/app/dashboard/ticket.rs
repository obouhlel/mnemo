//! Détail du ticket sélectionné et panneau des tickets liés à la note ouverte.

use super::*;

impl MnemoApp {
    pub(super) fn ticket_detail(&mut self, ui: &mut egui::Ui, t: &Ticket, acts: &mut Vec<DashAction>) {
        let p = self.palette();
        let fresh = !matches!(&self.dash.edit, Some(e) if e.id == t.id);
        let stale =
            matches!(&self.dash.edit, Some(e) if e.id == t.id && (e.base_title != t.title || e.base_body != t.body));
        let title_id = egui::Id::unique(("ticket-title", t.id));
        let body_id = egui::Id::unique(("ticket-body", t.id));
        let editing = ui.memory(|m| m.has_focus(title_id) || m.has_focus(body_id));
        if fresh || (stale && !editing) {
            self.dash.edit = Some(Edit {
                id: t.id,
                title: t.title.clone(),
                body: t.body.clone(),
                base_title: t.title.clone(),
                base_body: t.body.clone(),
            });
            self.dash.confirm_delete = None;
        }
        let project = self.board.projects.iter().find(|pr| pr.id == t.project_id).cloned();
        egui::ScrollArea::vertical().id_salt("ticket-detail-scroll").auto_shrink([false, false]).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("T-{}", t.number)).color(p.faint));
                if let Some(pr) = &project {
                    ui.label(RichText::new(&pr.name).color(p.muted));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button_sized(ui, "\u{e1b2}", "Fermer", false, true, 15.0).clicked() {
                        acts.push(DashAction::Select(None));
                    }
                });
            });
            let e = self.dash.edit.as_mut().unwrap();
            let r = ui.add(
                egui::TextEdit::multiline(&mut e.title)
                    .id(title_id)
                    .font(FontId::new(20.0, theme::bold_family()))
                    .frame(egui::Frame::NONE)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY),
            );
            if r.lost_focus() || (r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter))) {
                let clean = e.title.replace('\n', " ").trim().to_string();
                if !clean.is_empty() && clean != t.title {
                    e.base_title = clean.clone();
                    acts.push(DashAction::Patch(t.id, TicketPatch { title: Some(clean.clone()), ..Default::default() }));
                }
                e.title = if clean.is_empty() { t.title.clone() } else { clean };
                if r.has_focus() {
                    r.surrender_focus();
                }
            }
            ui.add_space(10.0);
            let field = |ui: &mut egui::Ui, label: &str| {
                ui.add_sized(egui::vec2(80.0, 22.0), egui::Label::new(RichText::new(label).color(p.muted)));
            };
            ui.horizontal(|ui| {
                field(ui, "Statut");
                let mut st = t.status;
                egui::ComboBox::from_id_salt("ticket-status").selected_text(RichText::new(st.label()).color(st.color())).show_ui(ui, |ui| {
                    for s in Status::ALL {
                        ui.selectable_value(&mut st, s, RichText::new(s.label()).color(s.color()));
                    }
                });
                if st != t.status {
                    acts.push(DashAction::Patch(t.id, TicketPatch { status: Some(st), ..Default::default() }));
                }
            });
            ui.horizontal(|ui| {
                field(ui, "Priorité");
                let mut pr = t.priority;
                egui::ComboBox::from_id_salt("ticket-priority").selected_text(PRIORITIES[pr.clamp(0, 3) as usize]).show_ui(ui, |ui| {
                    for (i, name) in PRIORITIES.iter().enumerate() {
                        ui.selectable_value(&mut pr, i as i16, *name);
                    }
                });
                if pr != t.priority {
                    acts.push(DashAction::Patch(t.id, TicketPatch { priority: Some(pr), ..Default::default() }));
                }
            });
            ui.horizontal(|ui| {
                field(ui, "Projet");
                let mut pid = t.project_id;
                let name = project.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                egui::ComboBox::from_id_salt("ticket-project").selected_text(name).show_ui(ui, |ui| {
                    for pr in &self.board.projects {
                        ui.selectable_value(&mut pid, pr.id, &pr.name);
                    }
                });
                if pid != t.project_id {
                    acts.push(DashAction::Patch(t.id, TicketPatch { project_id: Some(pid), ..Default::default() }));
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("Tags").color(p.muted));
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
                let mut all: Vec<String> = TAGS.iter().map(|t| t.0.to_string()).collect();
                for g in &t.tags {
                    if !all.contains(g) {
                        all.push(g.clone());
                    }
                }
                for g in all {
                    let on = t.tags.contains(&g);
                    if pill(ui, &g, tag_color(&g), on).on_hover_text(if on { "Retirer" } else { "Ajouter" }).clicked() {
                        let mut tags = t.tags.clone();
                        if on {
                            tags.retain(|x| x != &g);
                        } else {
                            tags.push(g.clone());
                        }
                        acts.push(DashAction::Patch(t.id, TicketPatch { tags: Some(tags), ..Default::default() }));
                    }
                }
                let r = ui.add(egui::TextEdit::singleline(&mut self.dash.new_tag).hint_text("+ tag").desired_width(80.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    let g = self.dash.new_tag.trim().to_string();
                    if !g.is_empty() && !t.tags.contains(&g) {
                        let mut tags = t.tags.clone();
                        tags.push(g);
                        acts.push(DashAction::Patch(t.id, TicketPatch { tags: Some(tags), ..Default::default() }));
                    }
                    self.dash.new_tag.clear();
                }
            });
            ui.add_space(10.0);
            ui.label(RichText::new("Description (Markdown)").color(p.muted));
            let e = self.dash.edit.as_mut().unwrap();
            let r = ui.add(egui::TextEdit::multiline(&mut e.body).id(body_id).desired_rows(6).desired_width(f32::INFINITY).hint_text("Contexte, critères d'acceptation…"));
            if r.lost_focus() && e.body != t.body {
                e.base_body = e.body.clone();
                acts.push(DashAction::Patch(t.id, TicketPatch { body: Some(e.body.clone()), ..Default::default() }));
            }

            // ---- reports
            ui.add_space(12.0);
            let links: Vec<ReportLink> = self.board.reports.iter().filter(|r| r.ticket_id == t.id).cloned().collect();
            ui.label(RichText::new(format!("Rapports ({})", links.len())).color(p.text).strong());
            ui.add_space(2.0);
            for l in &links {
                ui.horizontal(|ui| {
                    ui.label(theme::icon_text("\u{e0d0}", 13.0).color(p.muted));
                    if ui.add(egui::Label::new(RichText::new(basename(&l.path)).color(p.link)).sense(egui::Sense::click()).truncate())
                        .on_hover_text(&l.path)
                        .clicked()
                    {
                        acts.push(DashAction::OpenNote(l.note_id));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button_sized(ui, "\u{e19c}", "Délier ce rapport", false, true, 13.0).clicked() {
                            acts.push(DashAction::Unlink(t.id, l.note_id));
                        }
                    });
                });
            }
            ui.horizontal(|ui| {
                if ui.button("+ Nouveau rapport").clicked() {
                    acts.push(DashAction::NewReport(t.id));
                }
                let link_btn = ui.button("Lier une note…");
                egui::Popup::menu(&link_btn).show(|ui| {
                    ui.set_min_width(260.0);
                    ui.add(egui::TextEdit::singleline(&mut self.dash.link_query).hint_text("Rechercher une note…").desired_width(f32::INFINITY));
                    let q = fold_accents(&self.dash.link_query);
                    let mut shown = 0;
                    for n in &self.notes {
                        if shown >= 10 || links.iter().any(|l| l.note_id == n.id) || !fold_accents(&n.path).contains(&q) {
                            continue;
                        }
                        shown += 1;
                        if theme::suggestion_row(ui, basename(&n.path), parent(&n.path).unwrap_or(""), false).clicked() {
                            acts.push(DashAction::Link(t.id, n.id));
                            ui.close();
                        }
                    }
                    if shown == 0 {
                        ui.label(RichText::new("Aucune note").color(p.faint));
                    }
                });
            });

            // ---- GitHub
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.label(theme::icon_text("\u{e0e9}", 14.0).color(p.text));
                ui.label(RichText::new("GitHub").color(p.text).strong());
            });
            ui.add_space(2.0);
            match (t.github_number, project.as_ref().and_then(|pr| pr.github_repo.clone())) {
                (Some(n), repo) => {
                    let open = t.github_state.as_deref() != Some("closed");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Issue #{n}")).color(p.text));
                        pill(ui, if open { "ouverte" } else { "fermée" }, if open { Status::Done.color() } else { Color32::from_rgb(0x8b, 0x6c, 0xef) }, false);
                        if let Some(r) = &repo {
                            ui.label(RichText::new(r).color(p.faint).small());
                        }
                    });
                    if let Some(s) = t.github_synced_at {
                        ui.label(RichText::new(format!("Synchronisé le {}", s.with_timezone(&chrono::Local).format("%d/%m/%Y à %H:%M"))).small().color(p.faint));
                    }
                    ui.horizontal(|ui| {
                        if let Some(url) = &t.github_url
                            && ui.button("Ouvrir sur GitHub").clicked()
                        {
                            acts.push(DashAction::OpenUrl(url.clone()));
                        }
                        if ui.add_enabled(!self.dash.syncing.contains(&t.project_id), egui::Button::new("Synchroniser")).clicked() {
                            acts.push(DashAction::Sync(t.project_id));
                        }
                    });
                }
                (None, Some(repo)) => {
                    ui.label(RichText::new(format!("Pas encore publié sur {repo}.")).color(p.faint));
                    let busy = self.dash.publishing.contains(&t.id);
                    if ui.add_enabled(!busy, egui::Button::new(if busy { "Publication…" } else { "Publier sur GitHub" })).clicked() {
                        acts.push(DashAction::Publish(t.id));
                    }
                }
                (None, None) => {
                    ui.add(egui::Label::new(RichText::new("Associez un dépôt GitHub au projet (Modifier le projet) pour publier ce ticket en issue.").color(p.faint)).wrap());
                }
            }

            // ---- dates & delete
            ui.add_space(12.0);
            let fmt = |d: chrono::DateTime<chrono::Utc>| d.with_timezone(&chrono::Local).format("%d/%m/%Y %H:%M").to_string();
            ui.label(RichText::new(format!("Créé le {}", fmt(t.created_at))).small().color(p.faint));
            if let Some(d) = t.done_at {
                ui.label(RichText::new(format!("Terminé le {}", fmt(d))).small().color(p.faint));
            }
            ui.add_space(10.0);
            let confirm = self.dash.confirm_delete == Some(t.id);
            let label = if confirm { "Confirmer la suppression" } else { "Supprimer le ticket" };
            if ui.button(RichText::new(label).color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                if confirm {
                    acts.push(DashAction::Delete(t.id));
                } else {
                    self.dash.confirm_delete = Some(t.id);
                }
            }
            if confirm && t.github_number.is_some() {
                ui.label(RichText::new("L'issue GitHub n'est pas supprimée.").small().color(p.faint));
            }
        });
    }

    /// Right sidebar tab of a note: tickets for which this note is a report.
    pub(in crate::app) fn note_tickets_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette();
        let Some(note) = self.open.as_ref().map(|n| n.id) else { return };
        let linked: Vec<Ticket> = self
            .board
            .reports
            .iter()
            .filter(|r| r.note_id == note)
            .filter_map(|r| self.board.tickets.iter().find(|t| t.id == r.ticket_id).cloned())
            .collect();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Tickets liés").color(p.text).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(linked.len().to_string()).color(p.faint).small());
            });
        });
        ui.add_space(4.0);
        if linked.is_empty() {
            ui.label(RichText::new("Cette note n'est liée à aucun ticket.").color(p.faint));
        }
        let mut open_ticket = None;
        let mut unlink = None;
        for t in &linked {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                ui.painter().circle_filled(r.center(), 4.0, t.status.color());
                if ui
                    .add(
                        egui::Label::new(RichText::new(format!("T-{} {}", t.number, t.title)).color(p.link))
                            .sense(egui::Sense::click())
                            .truncate(),
                    )
                    .on_hover_text(t.status.label())
                    .clicked()
                {
                    open_ticket = Some(t.id);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::icon_button_sized(ui, "\u{e19c}", "Délier", false, true, 13.0).clicked() {
                        unlink = Some(t.id);
                    }
                });
            });
        }
        ui.add_space(6.0);
        let btn = ui.button("Lier à un ticket…");
        egui::Popup::menu(&btn).show(|ui| {
            ui.set_min_width(300.0);
            ui.add(
                egui::TextEdit::singleline(&mut self.dash.link_query)
                    .hint_text("Rechercher un ticket…")
                    .desired_width(f32::INFINITY),
            );
            let q = fold_accents(&self.dash.link_query);
            let mut shown = 0;
            for t in &self.board.tickets {
                if shown >= 10
                    || linked.iter().any(|l| l.id == t.id)
                    || !(fold_accents(&t.title).contains(&q) || format!("t-{}", t.number).contains(&q))
                {
                    continue;
                }
                shown += 1;
                let project = self
                    .board
                    .projects
                    .iter()
                    .find(|pr| pr.id == t.project_id)
                    .map(|pr| pr.name.as_str())
                    .unwrap_or("");
                if theme::suggestion_row(ui, &format!("T-{} {}", t.number, t.title), project, false).clicked() {
                    self.db.send(Cmd::LinkReport { ticket: t.id, note });
                    ui.close();
                }
            }
            if shown == 0 {
                ui.label(RichText::new("Aucun ticket").color(p.faint));
            }
        });
        if let Some(t) = unlink {
            self.board.reports.retain(|r| !(r.ticket_id == t && r.note_id == note));
            self.db.send(Cmd::UnlinkReport { ticket: t, note });
        }
        if let Some(t) = open_ticket {
            self.open_dashboard(Some(t));
        }
    }
}
