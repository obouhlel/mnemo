//! Application des actions du tableau de bord (DashAction) et dialogue de projet.

use super::*;

fn apply_patch(t: &mut Ticket, p: &TicketPatch) {
    if let Some(v) = &p.title {
        t.title = v.clone();
    }
    if let Some(v) = &p.body {
        t.body = v.clone();
    }
    if let Some(v) = p.status {
        t.status = v;
    }
    if let Some(v) = &p.tags {
        t.tags = v.clone();
    }
    if let Some(v) = p.priority {
        t.priority = v;
    }
    if let Some(v) = p.project_id {
        t.project_id = v;
    }
}

/// Path-safe text (note paths forbid [ ] | # ^ \ and "/" would create folders)
fn path_safe(s: &str) -> String {
    let cleaned: String = s.chars().map(|c| if "[]|#^\\/".contains(c) || c.is_control() { ' ' } else { c }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect()
}

impl MnemoApp {
    pub(super) fn apply_dash_actions(&mut self, ctx: &egui::Context, acts: Vec<DashAction>, now: f64) {
        for a in acts {
            match a {
                DashAction::Select(id) => self.dash.selected = id,
                DashAction::Create { project, title, tags, status } => {
                    let title = title.trim().to_string();
                    if !title.is_empty() {
                        self.db.send(Cmd::CreateTicket { project_id: project, title, tags, status });
                    }
                }
                DashAction::Patch(id, patch) => {
                    if let Some(t) = self.board.tickets.iter_mut().find(|t| t.id == id) {
                        apply_patch(t, &patch); // optimistic, the DB event confirms
                    }
                    self.db.send(Cmd::UpdateTicket { id, patch });
                }
                DashAction::Delete(id) => {
                    self.board.tickets.retain(|t| t.id != id);
                    self.dash.selected = None;
                    self.db.send(Cmd::DeleteTicket(id));
                }
                DashAction::OpenNote(id) => self.open_in(id, OpenMode::NewTab),
                DashAction::NewReport(id) => {
                    if let Some(t) = self.board.tickets.iter().find(|t| t.id == id) {
                        let project = self
                            .board
                            .projects
                            .iter()
                            .find(|pr| pr.id == t.project_id)
                            .map(|pr| pr.name.clone())
                            .unwrap_or_default();
                        let date = chrono::Local::now().format("%Y-%m-%d");
                        let path =
                            format!("Rapports/{}/T-{} {} ({date})", path_safe(&project), t.number, path_safe(&t.title));
                        let content = format!(
                            "**Ticket :** T-{} {} ({})\n**Projet :** {}\n**Tags :** {}\n\n## Contexte\n\n\n## Travail réalisé\n\n\n## Prochaines étapes\n- [ ] \n",
                            t.number,
                            t.title,
                            t.status.label(),
                            project,
                            t.tags.iter().map(|g| format!("#{}", g.replace(' ', "-"))).collect::<Vec<_>>().join(" "),
                        );
                        self.open_mode = OpenMode::NewTab;
                        self.db.send(Cmd::CreateReport { ticket: id, path, content });
                    }
                }
                DashAction::Link(t, n) => self.db.send(Cmd::LinkReport { ticket: t, note: n }),
                DashAction::Unlink(t, n) => {
                    self.board.reports.retain(|r| !(r.ticket_id == t && r.note_id == n));
                    self.db.send(Cmd::UnlinkReport { ticket: t, note: n });
                }
                DashAction::Publish(id) => {
                    if self.cfg.github_token.is_none() {
                        self.notice("Configurez d'abord un token dans Paramètres › GitHub", true, now);
                    } else {
                        self.dash.publishing.insert(id);
                        self.db.send(Cmd::GithubPublish(id));
                    }
                }
                DashAction::Sync(pid) => {
                    if self.cfg.github_token.is_none() {
                        self.notice("Configurez d'abord un token dans Paramètres › GitHub", true, now);
                    } else if self.dash.syncing.insert(pid) {
                        self.db.send(Cmd::GithubSync(pid));
                    }
                }
                DashAction::EditProject(id) => {
                    // liste des dépôts du compte pour les proposer (une fois, ou après une erreur)
                    if self.cfg.github_token.is_some() && !matches!(self.dash.repos, Some(Ok(_))) {
                        self.dash.repos = None;
                        self.db.send(Cmd::GithubRepos);
                    }
                    let pr = id.and_then(|id| self.board.projects.iter().find(|pr| pr.id == id));
                    self.dash.project_form = Some(ProjectForm {
                        id,
                        name: pr.map(|p| p.name.clone()).unwrap_or_default(),
                        description: pr.map(|p| p.description.clone()).unwrap_or_default(),
                        repo: pr.and_then(|p| p.github_repo.clone()).unwrap_or_default(),
                        error: None,
                        confirm_delete: false,
                    });
                }
                DashAction::Toggle(k) => {
                    if !self.dash.collapsed.remove(&k) {
                        self.dash.collapsed.insert(k);
                    }
                }
                DashAction::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            }
        }
    }

    pub(super) fn project_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.dash.project_form.take() else { return };
        let p = self.palette();
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("project-form"))
            .frame(
                egui::Frame::new()
                    .fill(p.bg2)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(10.0)
                    .inner_margin(20.0),
            )
            .show(ctx, |ui| {
                ui.set_width(440.0);
                ui.label(
                    RichText::new(if form.id.is_some() { "Modifier le projet" } else { "Nouveau projet" })
                        .size(18.0)
                        .strong()
                        .color(p.text),
                );
                ui.add_space(10.0);
                ui.label(RichText::new("Nom").color(p.muted));
                ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.label(RichText::new("Description").color(p.muted));
                ui.add(egui::TextEdit::multiline(&mut form.description).desired_rows(2).desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.label(RichText::new("Dépôt GitHub (facultatif)").color(p.muted));
                ui.add(
                    egui::TextEdit::singleline(&mut form.repo)
                        .hint_text("propriétaire/dépôt ou https://github.com/…")
                        .desired_width(f32::INFINITY),
                );
                if self.cfg.github_token.is_none() {
                    ui.label(
                        RichText::new("Renseignez un token dans Paramètres › GitHub pour choisir parmi vos dépôts.")
                            .small()
                            .color(p.faint),
                    );
                } else {
                    // dépôts du compte, filtrés par ce qui est tapé
                    match &self.dash.repos {
                        None => {
                            ui.label(RichText::new("Chargement de vos dépôts…").small().color(p.faint));
                        }
                        Some(Err(e)) => {
                            ui.label(RichText::new(format!("Dépôts indisponibles : {e}")).small().color(p.faint));
                        }
                        Some(Ok(repos)) => {
                            let q = form.repo.trim().to_lowercase();
                            let matches: Vec<&String> = repos
                                .iter()
                                .filter(|r| q.is_empty() || (r.to_lowercase().contains(&q) && r.to_lowercase() != q))
                                .take(6)
                                .collect();
                            if !matches.is_empty() {
                                ui.add_space(4.0);
                                egui::Frame::new().fill(p.bg).corner_radius(6.0).inner_margin(4.0).show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    for r in matches {
                                        if theme::tree_row(ui, r, 0, false, None).clicked() {
                                            if form.name.trim().is_empty() {
                                                form.name = r.rsplit('/').next().unwrap_or(r).to_string();
                                            }
                                            form.repo = r.clone();
                                        }
                                    }
                                });
                            }
                        }
                    }
                    if !form.repo.trim().is_empty() {
                        ui.label(
                            RichText::new(
                                "À l'enregistrement, les issues et pull requests existantes sont importées avec leurs labels.",
                            )
                            .small()
                            .color(p.faint),
                        );
                    }
                }
                if let Some(e) = &form.error {
                    ui.colored_label(Color32::from_rgb(0xfb, 0x46, 0x4c), e);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Enregistrer").clicked() {
                        let repo = form.repo.trim();
                        let repo =
                            if repo.is_empty() { Ok(None) } else { crate::github::normalize_repo(repo).map(Some) };
                        match (form.name.trim().is_empty(), repo) {
                            (true, _) => form.error = Some("Le nom est obligatoire".into()),
                            (_, Err(e)) => form.error = Some(e),
                            (false, Ok(repo)) => {
                                let (name, description) = (form.name.trim().to_string(), form.description.clone());
                                self.db.send(match form.id {
                                    Some(id) => Cmd::UpdateProject { id, name, description, repo },
                                    None => Cmd::CreateProject { name, description, repo },
                                });
                                close = true;
                            }
                        }
                    }
                    if ui.button("Annuler").clicked() {
                        close = true;
                    }
                    if let Some(id) = form.id {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let label = if form.confirm_delete {
                                "Confirmer : supprimer projet et tickets"
                            } else {
                                "Supprimer"
                            };
                            if ui.button(RichText::new(label).color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                                if form.confirm_delete {
                                    self.db.send(Cmd::DeleteProject(id));
                                    self.dash.project = None;
                                    close = true;
                                } else {
                                    form.confirm_delete = true;
                                }
                            }
                        });
                    }
                });
            });
        if modal.should_close() {
            close = true;
        }
        if !close {
            self.dash.project_form = Some(form);
        }
    }
}
