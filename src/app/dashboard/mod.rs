//! Dashboard tab: projects, tickets by tag (rows) or by status (kanban columns), ticket details,
//! reports (notes linked to tickets) and GitHub synchronisation.

use super::*;
use crate::board::*;
use std::collections::HashSet;

mod actions;
mod columns;
mod rows;
mod ticket;

#[derive(Clone, Copy, PartialEq, Default)]
pub(super) enum DashMode {
    #[default]
    Rows,
    Columns,
}

pub(super) struct DashState {
    pub project: Option<Uuid>,
    pub mode: DashMode,
    pub collapsed: HashSet<String>,
    pub selected: Option<Uuid>,
    pub show_done: bool,
    pub filter: String,
    pub new_titles: HashMap<String, String>,
    pub edit: Option<Edit>,
    pub project_form: Option<ProjectForm>,
    pub link_query: String,
    pub new_tag: String,
    pub syncing: HashSet<Uuid>,
    pub publishing: HashSet<Uuid>,
    pub confirm_delete: Option<Uuid>,
    pub github_user: Option<Result<String, String>>,
    /// dépôts visibles avec le token (None : pas encore demandés / en cours)
    pub repos: Option<Result<Vec<String>, String>>,
    pub last_auto_sync: f64,
}

impl Default for DashState {
    fn default() -> Self {
        Self {
            project: None,
            mode: DashMode::Rows,
            collapsed: HashSet::new(),
            selected: None,
            show_done: true,
            filter: String::new(),
            new_titles: HashMap::new(),
            edit: None,
            project_form: None,
            link_query: String::new(),
            new_tag: String::new(),
            syncing: HashSet::new(),
            publishing: HashSet::new(),
            confirm_delete: None,
            github_user: None,
            repos: None,
            last_auto_sync: 0.0,
        }
    }
}

/// Edit buffers of the selected ticket (committed on focus loss)
pub(super) struct Edit {
    id: Uuid,
    title: String,
    body: String,
    base_title: String,
    base_body: String,
}

pub(super) struct ProjectForm {
    id: Option<Uuid>,
    name: String,
    description: String,
    repo: String,
    error: Option<String>,
    confirm_delete: bool,
}

/// Action produite par l'interface du tableau de bord, appliquée en fin de frame par `apply_dash_actions`.
enum DashAction {
    Select(Option<Uuid>),
    Create { project: Uuid, title: String, tags: Vec<String>, status: Status },
    Patch(Uuid, TicketPatch),
    Delete(Uuid),
    OpenNote(Uuid),
    NewReport(Uuid),
    Link(Uuid, Uuid),
    Unlink(Uuid, Uuid),
    Publish(Uuid),
    Sync(Uuid),
    EditProject(Option<Uuid>),
    Toggle(String),
    OpenUrl(String),
}

fn pill(ui: &mut egui::Ui, text: &str, color: Color32, filled: bool) -> egui::Response {
    let font = FontId::proportional(11.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, if filled { Color32::WHITE } else { color });
    let size = galley.size() + egui::vec2(14.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let bg = if filled { color } else { color.gamma_multiply(0.16) };
    ui.painter().rect_filled(rect, 9.0, bg);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

fn progress_bar(ui: &mut egui::Ui, p: &theme::Palette, done: usize, total: usize, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 6.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, p.border);
    if total > 0 && done > 0 {
        let mut r = rect;
        r.set_width(rect.width() * done as f32 / total as f32);
        ui.painter().rect_filled(r, 3.0, if done == total { Status::Done.color() } else { p.accent });
    }
}

impl MnemoApp {
    pub(super) fn open_dashboard(&mut self, select: Option<Uuid>) {
        self.flush_save();
        match self.tabs.iter().position(|t| t.dashboard) {
            Some(i) => self.active_tab = i,
            None => {
                self.tabs.push(Tab { dashboard: true, ..Default::default() });
                self.active_tab = self.tabs.len() - 1;
            }
        }
        self.open = None;
        if let Some(id) = select {
            self.dash.selected = Some(id);
            if let Some(t) = self.board.tickets.iter().find(|t| t.id == id) {
                self.dash.project = Some(t.project_id);
            }
        }
        self.need.board = true;
    }

    fn scoped_tickets(&self, project: Uuid) -> Vec<&Ticket> {
        let f = fold_accents(self.dash.filter.trim());
        self.board
            .tickets
            .iter()
            .filter(|t| t.project_id == project)
            .filter(|t| self.dash.show_done || t.status != Status::Done)
            .filter(|t| {
                f.is_empty()
                    || fold_accents(&t.title).contains(&f)
                    || fold_accents(&t.body).contains(&f)
                    || format!("t-{}", t.number).contains(&f)
                    || t.tags.iter().any(|g| fold_accents(g).contains(&f))
            })
            .collect()
    }

    pub(super) fn dashboard_ui(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.palette();
        // projets et tickets sont partagés (et synchronisés avec GitHub) : ils vivent dans la base
        if self.conn != ConnState::Connected {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() * 0.3).max(20.0));
                ui.label(RichText::new("Tableau de bord").size(22.0).color(p.muted));
                ui.add_space(10.0);
                let msg = if self.cfg.database_url.is_empty() {
                    "Les projets et tickets sont partagés entre vos appareils : ils ont besoin d'une base de synchronisation."
                } else {
                    "La base de synchronisation est injoignable pour le moment ; le tableau de bord revient dès la reconnexion."
                };
                ui.label(RichText::new(msg).color(p.faint));
                ui.add_space(12.0);
                if self.cfg.database_url.is_empty() && ui.button("Connecter une base…").clicked() {
                    self.settings_tab = SettingsTab::Connection;
                    self.show_settings = true;
                }
            });
            return;
        }
        let mut acts: Vec<DashAction> = vec![];
        // auto-sync with GitHub every 5 minutes
        if self.cfg.github_auto_sync && self.cfg.github_token.is_some() && now - self.dash.last_auto_sync > 300.0 {
            self.dash.last_auto_sync = now;
            for pr in self.board.projects.iter().filter(|pr| pr.github_repo.is_some()) {
                if self.dash.syncing.insert(pr.id) {
                    self.db.send(Cmd::GithubSync(pr.id));
                }
            }
        }
        if let Some(pid) = self.dash.project
            && !self.board.projects.iter().any(|pr| pr.id == pid)
            && !self.board.projects.is_empty()
        {
            self.dash.project = None;
        }

        // ---- view header (like a note's)
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        let project_name = self
            .dash
            .project
            .and_then(|id| self.board.projects.iter().find(|pr| pr.id == id))
            .map(|pr| pr.name.clone());
        let crumb = match &project_name {
            Some(n) => format!("Tableau de bord  /  {n}"),
            None => "Tableau de bord".to_string(),
        };
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, crumb, FontId::proportional(14.0), p.text);

        // ---- toolbar
        egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 2, bottom: 6 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                let label = project_name.clone().unwrap_or_else(|| "Tous les projets".into());
                egui::ComboBox::from_id_salt("dash-project").width(200.0).selected_text(label).show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.dash.project, None, "Tous les projets");
                    for pr in &self.board.projects {
                        ui.selectable_value(&mut self.dash.project, Some(pr.id), &pr.name);
                    }
                });
                ui.add_space(6.0);
                ui.selectable_value(&mut self.dash.mode, DashMode::Rows, "Lignes")
                    .on_hover_text("Todo list par projet, regroupée par tag");
                ui.selectable_value(&mut self.dash.mode, DashMode::Columns, "Colonnes")
                    .on_hover_text("Kanban par statut (glisser-déposer)");
                ui.add_space(6.0);
                ui.checkbox(&mut self.dash.show_done, "Terminés");
                ui.add(egui::TextEdit::singleline(&mut self.dash.filter).hint_text("Filtrer…").desired_width(170.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if theme::icon_button_sized(ui, "\u{e0de}", "Nouveau projet", false, true, 16.0).clicked() {
                        acts.push(DashAction::EditProject(None));
                    }
                    if let Some(pid) = self.dash.project {
                        if theme::icon_button_sized(
                            ui,
                            icon::SETTINGS,
                            "Modifier le projet (nom, description, dépôt GitHub)",
                            false,
                            true,
                            16.0,
                        )
                        .clicked()
                        {
                            acts.push(DashAction::EditProject(Some(pid)));
                        }
                        if self.board.projects.iter().any(|pr| pr.id == pid && pr.github_repo.is_some()) {
                            if self.dash.syncing.contains(&pid) {
                                ui.spinner();
                            } else if theme::icon_button_sized(
                                ui,
                                icon::SYNC,
                                "Synchroniser avec GitHub",
                                false,
                                true,
                                16.0,
                            )
                            .clicked()
                            {
                                acts.push(DashAction::Sync(pid));
                            }
                        }
                    }
                });
            });
        });

        // ---- summary
        let scope: Vec<&Ticket> = match self.dash.project {
            Some(pid) => self.board.tickets.iter().filter(|t| t.project_id == pid).collect(),
            None => self.board.tickets.iter().collect(),
        };
        let (done, total) = progress(scope.iter().copied());
        egui::Frame::new().inner_margin(egui::Margin { left: 20, right: 20, top: 0, bottom: 10 }).show(ui, |ui| {
            ui.horizontal(|ui| {
                progress_bar(ui, &p, done, total, 220.0);
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                ui.label(RichText::new(format!("{done} / {total} terminés · {pct} %")).color(p.text));
                ui.add_space(10.0);
                for st in Status::ALL {
                    let n = scope.iter().filter(|t| t.status == st).count();
                    if n > 0 {
                        pill(ui, &format!("{} {n}", st.label()), st.color(), false);
                    }
                }
            });
        });
        let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, p.border));

        // ---- body: list / kanban + ticket detail
        let selected = self.dash.selected.and_then(|id| self.board.tickets.iter().find(|t| t.id == id).cloned());
        if let Some(t) = &selected {
            egui::Panel::right("ticket-detail")
                .default_size(400.0)
                .size_range(320.0..=640.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(p.bg2).inner_margin(egui::Margin::symmetric(16, 12)))
                .show(ui, |ui| self.ticket_detail(ui, t, &mut acts));
        }
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
            if self.board.projects.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.add_space(60.0);
                    ui.label(RichText::new("Aucun projet pour l'instant").size(20.0).color(p.muted));
                    ui.add_space(8.0);
                    ui.label(RichText::new("Créez un projet pour suivre ses tickets (Feature, Bug…), écrire des rapports et le relier à GitHub.").color(p.faint));
                    ui.add_space(14.0);
                    if ui.button("+ Créer un projet").clicked() {
                        acts.push(DashAction::EditProject(None));
                    }
                });
                return;
            }
            match self.dash.mode {
                DashMode::Rows => self.rows_view(ui, &mut acts),
                DashMode::Columns => self.columns_view(ui, &mut acts),
            }
        });
        self.project_dialog(ui.ctx());
        self.apply_dash_actions(ui.ctx(), acts, now);
    }

    // ------------------------------------------------------------------ rows (todo list)
    // ------------------------------------------------------------------ columns (kanban)
    // ------------------------------------------------------------------ ticket detail
    // ------------------------------------------------------------------ project dialog
    // ------------------------------------------------------------------ actions
}
