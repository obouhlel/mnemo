//! Dashboard tab: projects, tickets by tag (rows) or by status (kanban columns), ticket details,
//! reports (notes linked to tickets) and GitHub synchronisation.

use super::*;
use crate::board::*;
use std::collections::HashSet;

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

enum DA {
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
        let f = fold(self.dash.filter.trim());
        self.board
            .tickets
            .iter()
            .filter(|t| t.project_id == project)
            .filter(|t| self.dash.show_done || t.status != Status::Done)
            .filter(|t| {
                f.is_empty()
                    || fold(&t.title).contains(&f)
                    || fold(&t.body).contains(&f)
                    || format!("t-{}", t.number).contains(&f)
                    || t.tags.iter().any(|g| fold(g).contains(&f))
            })
            .collect()
    }

    pub(super) fn dashboard_ui(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.pal();
        let mut acts: Vec<DA> = vec![];
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
                        acts.push(DA::EditProject(None));
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
                            acts.push(DA::EditProject(Some(pid)));
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
                                acts.push(DA::Sync(pid));
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
                        acts.push(DA::EditProject(None));
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
    fn rows_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<DA>) {
        let p = self.pal();
        let projects: Vec<Project> = match self.dash.project {
            Some(pid) => self.board.projects.iter().filter(|pr| pr.id == pid).cloned().collect(),
            None => self.board.projects.clone(),
        };
        let single = self.dash.project.is_some();
        egui::ScrollArea::vertical().id_salt("dash-rows").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 8, bottom: 60 }).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for pr in &projects {
                    let owned: Vec<Ticket> = self.scoped_tickets(pr.id).into_iter().cloned().collect();
                    let tickets: Vec<&Ticket> = owned.iter().collect();
                    let pkey = format!("p:{}", pr.id);
                    let mut depth = 0;
                    if !single {
                        let all: Vec<&Ticket> = self.board.tickets.iter().filter(|t| t.project_id == pr.id).collect();
                        let (d, n) = progress(all.into_iter());
                        let open = !self.dash.collapsed.contains(&pkey);
                        let r = group_row(ui, &p, &pr.name, None, d, n, open, 0, true, pr.github_repo.as_deref());
                        if r.clicked() {
                            acts.push(DA::Toggle(pkey.clone()));
                        }
                        if r.double_clicked() {
                            self.dash.project = Some(pr.id);
                        }
                        if !open {
                            continue;
                        }
                        depth = 1;
                    }
                    // tag groups: built-in tags first, then custom ones, then untagged
                    let mut groups: Vec<String> = TAGS
                        .iter()
                        .map(|t| t.0.to_string())
                        .filter(|tag| tickets.iter().any(|t| t.tags.contains(tag)))
                        .collect();
                    let mut custom: Vec<String> = tickets
                        .iter()
                        .flat_map(|t| t.tags.iter().cloned())
                        .filter(|g| !groups.contains(g) && !TAGS.iter().any(|t| t.0 == g))
                        .collect();
                    custom.sort();
                    custom.dedup();
                    groups.extend(custom);
                    if groups.is_empty() {
                        groups.extend(["Feature".to_string(), "Bug".to_string()]);
                    }
                    let untagged: Vec<&Ticket> = tickets.iter().copied().filter(|t| t.tags.is_empty()).collect();
                    let mut keys: Vec<(String, Vec<&Ticket>)> = groups
                        .into_iter()
                        .map(|g| {
                            let v = tickets.iter().copied().filter(|t| t.tags.contains(&g)).collect();
                            (g, v)
                        })
                        .collect();
                    if !untagged.is_empty() {
                        keys.push((String::new(), untagged));
                    }
                    for (tag, list) in keys {
                        let gkey = format!("t:{}:{tag}", pr.id);
                        let open = !self.dash.collapsed.contains(&gkey);
                        let (d, n) = progress(list.iter().copied());
                        let label = if tag.is_empty() { "Sans tag".to_string() } else { tag.clone() };
                        let r = group_row(
                            ui,
                            &p,
                            &label,
                            Some(if tag.is_empty() { p.faint } else { tag_color(&tag) }),
                            d,
                            n,
                            open,
                            depth,
                            false,
                            None,
                        );
                        if r.clicked() {
                            acts.push(DA::Toggle(gkey.clone()));
                        }
                        if !open {
                            continue;
                        }
                        for t in &list {
                            let reports = self.board.reports.iter().filter(|r| r.ticket_id == t.id).count();
                            self.ticket_row(ui, &p, t, reports, depth + 1, &gkey, acts);
                        }
                        // inline "add a ticket" row
                        let buf = self.dash.new_titles.entry(gkey.clone()).or_default();
                        ui.add_space(2.0);
                        ui.horizontal(|ui| {
                            ui.add_space(14.0 + (depth + 1) as f32 * 20.0);
                            let hint = if tag.is_empty() {
                                "+ Ajouter un ticket…".to_string()
                            } else {
                                format!("+ Ajouter un ticket « {tag} »…")
                            };
                            let r = ui.add(
                                egui::TextEdit::singleline(buf)
                                    .hint_text(RichText::new(hint).color(p.faint))
                                    .frame(egui::Frame::NONE)
                                    .desired_width(ui.available_width() - 10.0),
                            );
                            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !buf.trim().is_empty() {
                                acts.push(DA::Create {
                                    project: pr.id,
                                    title: std::mem::take(buf),
                                    tags: if tag.is_empty() { vec![] } else { vec![tag.clone()] },
                                    status: Status::Todo,
                                });
                                r.request_focus();
                            }
                        });
                        ui.add_space(6.0);
                    }
                    ui.add_space(10.0);
                }
            });
        });
    }

    #[allow(clippy::too_many_arguments)]
    /// `salt`: the group key (a ticket with several tags is listed in several groups)
    #[allow(clippy::too_many_arguments)]
    fn ticket_row(
        &self,
        ui: &mut egui::Ui,
        p: &theme::Palette,
        t: &Ticket,
        reports: usize,
        depth: usize,
        salt: &str,
        acts: &mut Vec<DA>,
    ) {
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
        let painter = ui.painter_at(rect);
        let selected = self.dash.selected == Some(t.id);
        if selected {
            painter.rect_filled(rect.shrink2(egui::vec2(2.0, 1.0)), 5.0, p.active);
        } else if resp.hovered() {
            painter.rect_filled(rect.shrink2(egui::vec2(2.0, 1.0)), 5.0, p.hover);
        }
        let x0 = rect.left() + 14.0 + depth as f32 * 20.0;
        let cy = rect.center().y;
        // checkbox
        let cb = egui::Rect::from_center_size(egui::pos2(x0 + 7.0, cy), egui::vec2(15.0, 15.0));
        let done = t.status == Status::Done;
        if done {
            painter.rect_filled(cb, 4.0, p.accent);
            let s = egui::Stroke::new(1.8, Color32::WHITE);
            let c = cb.center();
            painter.line_segment([c + egui::vec2(-3.8, 0.2), c + egui::vec2(-1.0, 3.0)], s);
            painter.line_segment([c + egui::vec2(-1.0, 3.0), c + egui::vec2(4.0, -2.8)], s);
        } else {
            painter.rect_stroke(
                cb,
                4.0,
                egui::Stroke::new(1.3, if t.status == Status::Todo { p.faint } else { t.status.color() }),
                egui::StrokeKind::Inside,
            );
        }
        let cb_resp = ui.interact(cb.expand(4.0), egui::Id::unique(("ticket-cb", t.id, salt)), egui::Sense::click());
        if cb_resp.clicked() {
            acts.push(DA::Patch(
                t.id,
                TicketPatch { status: Some(if done { Status::Todo } else { Status::Done }), ..Default::default() },
            ));
        }
        // number + title
        let mut x = x0 + 24.0;
        let num = painter.layout_no_wrap(format!("T-{}", t.number), FontId::proportional(12.5), p.faint);
        painter.galley(egui::pos2(x, cy - num.size().y / 2.0), num.clone(), p.faint);
        x += num.size().x + 10.0;
        // right side metadata (painted right to left)
        let mut right = rect.right() - 10.0;
        let mut meta = |text: String, color: Color32, family: Option<egui::FontFamily>| {
            let font = match family {
                Some(f) => FontId::new(13.0, f),
                None => FontId::proportional(12.0),
            };
            let g = painter.layout_no_wrap(text, font, color);
            right -= g.size().x;
            painter.galley(egui::pos2(right, cy - g.size().y / 2.0), g, color);
            right -= 12.0;
        };
        meta(t.status.label().to_string(), t.status.color(), None);
        if let Some(n) = t.github_number {
            let closed = t.github_state.as_deref() == Some("closed");
            meta(format!("#{n}"), if closed { p.faint } else { Status::Done.color() }, None);
            meta("\u{e0e9}".into(), p.faint, Some(theme::icon_family()));
        }
        if reports > 0 {
            meta(reports.to_string(), p.muted, None);
            meta("\u{e0d0}".into(), p.muted, Some(theme::icon_family()));
        }
        if let Some(c) = priority_color(t.priority) {
            meta(PRIORITIES[t.priority as usize].to_string(), c, None);
        }
        let title_w = (right - x - 8.0).max(40.0);
        let mut job = LayoutJob::single_section(
            t.title.clone(),
            TextFormat {
                font_id: FontId::proportional(14.0),
                color: if done { p.faint } else { p.text },
                strikethrough: if done { egui::Stroke::new(1.0, p.faint) } else { egui::Stroke::NONE },
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(title_w);
        let g = painter.layout_job(job);
        painter.galley(egui::pos2(x, cy - g.size().y / 2.0), g, p.text);
        if resp.clicked() && !cb_resp.clicked() {
            acts.push(DA::Select(if selected { None } else { Some(t.id) }));
        }
        resp.context_menu(|ui| {
            ui.label(RichText::new("Changer le statut").small().color(p.faint));
            for st in Status::ALL {
                if ui.selectable_label(t.status == st, st.label()).clicked() {
                    acts.push(DA::Patch(t.id, TicketPatch { status: Some(st), ..Default::default() }));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Nouveau rapport").clicked() {
                acts.push(DA::NewReport(t.id));
                ui.close();
            }
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                acts.push(DA::Delete(t.id));
                ui.close();
            }
        });
    }

    // ------------------------------------------------------------------ columns (kanban)
    fn columns_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<DA>) {
        let p = self.pal();
        let statuses: Vec<Status> =
            Status::ALL.into_iter().filter(|s| self.dash.show_done || *s != Status::Done).collect();
        let projects: Vec<Uuid> = match self.dash.project {
            Some(pid) => vec![pid],
            None => self.board.projects.iter().map(|pr| pr.id).collect(),
        };
        let tickets: Vec<Ticket> =
            projects.iter().flat_map(|pid| self.scoped_tickets(*pid).into_iter().cloned()).collect();
        let names: HashMap<Uuid, String> = self.board.projects.iter().map(|pr| (pr.id, pr.name.clone())).collect();
        let show_project = self.dash.project.is_none();
        egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 12, top: 10, bottom: 10 }).show(ui, |ui| {
            ui.columns(statuses.len(), |cols| {
                for (ci, st) in statuses.iter().enumerate() {
                    let ui = &mut cols[ci];
                    let col: Vec<&Ticket> = tickets.iter().filter(|t| t.status == *st).collect();
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, st.color());
                        ui.label(RichText::new(st.label()).strong().color(p.text));
                        ui.label(RichText::new(col.len().to_string()).color(p.faint));
                    });
                    ui.add_space(4.0);
                    let frame = egui::Frame::new().fill(p.bg2).corner_radius(8.0).inner_margin(6.0);
                    let (_, dropped) = ui.dnd_drop_zone::<Uuid, ()>(frame, |ui| {
                        ui.set_min_height((ui.available_height() - 10.0).max(120.0));
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical().id_salt(("kanban", ci)).auto_shrink([false, false]).show(
                            ui,
                            |ui| {
                                if *st == Status::Todo
                                    && let Some(pid) = self.dash.project
                                {
                                    let buf = self.dash.new_titles.entry(format!("k:{pid}")).or_default();
                                    let r = ui.add(
                                        egui::TextEdit::singleline(buf)
                                            .hint_text("+ Nouveau ticket…")
                                            .desired_width(f32::INFINITY),
                                    );
                                    if r.lost_focus()
                                        && ui.input(|i| i.key_pressed(Key::Enter))
                                        && !buf.trim().is_empty()
                                    {
                                        acts.push(DA::Create {
                                            project: pid,
                                            title: std::mem::take(buf),
                                            tags: vec![],
                                            status: Status::Todo,
                                        });
                                        r.request_focus();
                                    }
                                    ui.add_space(4.0);
                                }
                                for t in col {
                                    let reports = self.board.reports.iter().filter(|r| r.ticket_id == t.id).count();
                                    let selected = self.dash.selected == Some(t.id);
                                    let id = egui::Id::unique(("card", t.id));
                                    let r = ui.dnd_drag_source(id, t.id, |ui| {
                                        egui::Frame::new()
                                            .fill(p.bg)
                                            .stroke(egui::Stroke::new(1.0, if selected { p.accent } else { p.border }))
                                            .corner_radius(6.0)
                                            .inner_margin(egui::Margin::symmetric(10, 8))
                                            .show(ui, |ui| {
                                                ui.set_width(ui.available_width());
                                                // left-aligned (the column layout would justify the text)
                                                ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                                    ui.add(
                                                        egui::Label::new(RichText::new(&t.title).color(p.text)).wrap(),
                                                    );
                                                    ui.add_space(4.0);
                                                    ui.horizontal_wrapped(|ui| {
                                                        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
                                                        ui.label(
                                                            RichText::new(format!("T-{}", t.number))
                                                                .size(11.5)
                                                                .color(p.faint),
                                                        );
                                                        if show_project && let Some(n) = names.get(&t.project_id) {
                                                            ui.label(RichText::new(n).size(11.5).color(p.muted));
                                                        }
                                                        for g in &t.tags {
                                                            pill(ui, g, tag_color(g), false);
                                                        }
                                                        if let Some(n) = t.github_number {
                                                            ui.label(theme::icon_text("\u{e0e9}", 12.0).color(p.faint));
                                                            ui.label(
                                                                RichText::new(format!("#{n}"))
                                                                    .size(11.5)
                                                                    .color(p.faint),
                                                            );
                                                        }
                                                        if reports > 0 {
                                                            ui.label(theme::icon_text("\u{e0d0}", 12.0).color(p.faint));
                                                            ui.label(
                                                                RichText::new(reports.to_string())
                                                                    .size(11.5)
                                                                    .color(p.faint),
                                                            );
                                                        }
                                                        if let Some(c) = priority_color(t.priority) {
                                                            ui.label(
                                                                RichText::new(PRIORITIES[t.priority as usize])
                                                                    .size(11.5)
                                                                    .color(c),
                                                            );
                                                        }
                                                    });
                                                });
                                            })
                                            .response
                                    });
                                    let click = ui.interact(r.response.rect, id.with("click"), egui::Sense::click());
                                    if click.clicked() {
                                        acts.push(DA::Select(if selected { None } else { Some(t.id) }));
                                    }
                                    ui.add_space(6.0);
                                }
                            },
                        );
                    });
                    if let Some(id) = dropped {
                        acts.push(DA::Patch(*id, TicketPatch { status: Some(*st), ..Default::default() }));
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------------ ticket detail
    fn ticket_detail(&mut self, ui: &mut egui::Ui, t: &Ticket, acts: &mut Vec<DA>) {
        let p = self.pal();
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
                        acts.push(DA::Select(None));
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
                    acts.push(DA::Patch(t.id, TicketPatch { title: Some(clean.clone()), ..Default::default() }));
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
                    acts.push(DA::Patch(t.id, TicketPatch { status: Some(st), ..Default::default() }));
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
                    acts.push(DA::Patch(t.id, TicketPatch { priority: Some(pr), ..Default::default() }));
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
                    acts.push(DA::Patch(t.id, TicketPatch { project_id: Some(pid), ..Default::default() }));
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
                        acts.push(DA::Patch(t.id, TicketPatch { tags: Some(tags), ..Default::default() }));
                    }
                }
                let r = ui.add(egui::TextEdit::singleline(&mut self.dash.new_tag).hint_text("+ tag").desired_width(80.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    let g = self.dash.new_tag.trim().to_string();
                    if !g.is_empty() && !t.tags.contains(&g) {
                        let mut tags = t.tags.clone();
                        tags.push(g);
                        acts.push(DA::Patch(t.id, TicketPatch { tags: Some(tags), ..Default::default() }));
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
                acts.push(DA::Patch(t.id, TicketPatch { body: Some(e.body.clone()), ..Default::default() }));
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
                        acts.push(DA::OpenNote(l.note_id));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if theme::icon_button_sized(ui, "\u{e19c}", "Délier ce rapport", false, true, 13.0).clicked() {
                            acts.push(DA::Unlink(t.id, l.note_id));
                        }
                    });
                });
            }
            ui.horizontal(|ui| {
                if ui.button("+ Nouveau rapport").clicked() {
                    acts.push(DA::NewReport(t.id));
                }
                let link_btn = ui.button("Lier une note…");
                egui::Popup::menu(&link_btn).show(|ui| {
                    ui.set_min_width(260.0);
                    ui.add(egui::TextEdit::singleline(&mut self.dash.link_query).hint_text("Rechercher une note…").desired_width(f32::INFINITY));
                    let q = fold(&self.dash.link_query);
                    let mut shown = 0;
                    for n in &self.notes {
                        if shown >= 10 || links.iter().any(|l| l.note_id == n.id) || !fold(&n.path).contains(&q) {
                            continue;
                        }
                        shown += 1;
                        if theme::suggestion_row(ui, basename(&n.path), parent(&n.path).unwrap_or(""), false).clicked() {
                            acts.push(DA::Link(t.id, n.id));
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
                            acts.push(DA::OpenUrl(url.clone()));
                        }
                        if ui.add_enabled(!self.dash.syncing.contains(&t.project_id), egui::Button::new("Synchroniser")).clicked() {
                            acts.push(DA::Sync(t.project_id));
                        }
                    });
                }
                (None, Some(repo)) => {
                    ui.label(RichText::new(format!("Pas encore publié sur {repo}.")).color(p.faint));
                    let busy = self.dash.publishing.contains(&t.id);
                    if ui.add_enabled(!busy, egui::Button::new(if busy { "Publication…" } else { "Publier sur GitHub" })).clicked() {
                        acts.push(DA::Publish(t.id));
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
                    acts.push(DA::Delete(t.id));
                } else {
                    self.dash.confirm_delete = Some(t.id);
                }
            }
            if confirm && t.github_number.is_some() {
                ui.label(RichText::new("L'issue GitHub n'est pas supprimée.").small().color(p.faint));
            }
        });
    }

    // ------------------------------------------------------------------ project dialog
    fn project_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut form) = self.dash.project_form.take() else { return };
        let p = self.pal();
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
                if self.cfg.github_token.is_none() && !form.repo.trim().is_empty() {
                    ui.label(
                        RichText::new("Pensez à renseigner un token dans Paramètres › GitHub.").small().color(p.faint),
                    );
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

    // ------------------------------------------------------------------ actions
    fn apply_dash_actions(&mut self, ctx: &egui::Context, acts: Vec<DA>, now: f64) {
        for a in acts {
            match a {
                DA::Select(id) => self.dash.selected = id,
                DA::Create { project, title, tags, status } => {
                    let title = title.trim().to_string();
                    if !title.is_empty() {
                        self.db.send(Cmd::CreateTicket { project_id: project, title, tags, status });
                    }
                }
                DA::Patch(id, patch) => {
                    if let Some(t) = self.board.tickets.iter_mut().find(|t| t.id == id) {
                        apply_patch(t, &patch); // optimistic, the DB event confirms
                    }
                    self.db.send(Cmd::UpdateTicket { id, patch });
                }
                DA::Delete(id) => {
                    self.board.tickets.retain(|t| t.id != id);
                    self.dash.selected = None;
                    self.db.send(Cmd::DeleteTicket(id));
                }
                DA::OpenNote(id) => self.open_in(id, OpenMode::NewTab),
                DA::NewReport(id) => {
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
                DA::Link(t, n) => self.db.send(Cmd::LinkReport { ticket: t, note: n }),
                DA::Unlink(t, n) => {
                    self.board.reports.retain(|r| !(r.ticket_id == t && r.note_id == n));
                    self.db.send(Cmd::UnlinkReport { ticket: t, note: n });
                }
                DA::Publish(id) => {
                    if self.cfg.github_token.is_none() {
                        self.notice("Configurez d'abord un token dans Paramètres › GitHub", true, now);
                    } else {
                        self.dash.publishing.insert(id);
                        self.db.send(Cmd::GithubPublish(id));
                    }
                }
                DA::Sync(pid) => {
                    if self.cfg.github_token.is_none() {
                        self.notice("Configurez d'abord un token dans Paramètres › GitHub", true, now);
                    } else if self.dash.syncing.insert(pid) {
                        self.db.send(Cmd::GithubSync(pid));
                    }
                }
                DA::EditProject(id) => {
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
                DA::Toggle(k) => {
                    if !self.dash.collapsed.remove(&k) {
                        self.dash.collapsed.insert(k);
                    }
                }
                DA::OpenUrl(url) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            }
        }
    }

    /// Right sidebar tab of a note: tickets for which this note is a report.
    pub(super) fn note_tickets_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
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
            let q = fold(&self.dash.link_query);
            let mut shown = 0;
            for t in &self.board.tickets {
                if shown >= 10
                    || linked.iter().any(|l| l.id == t.id)
                    || !(fold(&t.title).contains(&q) || format!("t-{}", t.number).contains(&q))
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

/// Collapsible group row (project or tag) with a mini progress bar.
#[allow(clippy::too_many_arguments)]
fn group_row(
    ui: &mut egui::Ui,
    p: &theme::Palette,
    label: &str,
    dot: Option<Color32>,
    done: usize,
    total: usize,
    open: bool,
    depth: usize,
    strong: bool,
    repo: Option<&str>,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(width, if strong { 34.0 } else { 30.0 }), egui::Sense::click());
    let painter = ui.painter_at(rect);
    if resp.hovered() {
        painter.rect_filled(rect.shrink2(egui::vec2(2.0, 1.0)), 5.0, p.hover);
    }
    let cy = rect.center().y;
    let mut x = rect.left() + 8.0 + depth as f32 * 20.0;
    painter.text(
        egui::pos2(x + 6.0, cy),
        egui::Align2::CENTER_CENTER,
        if open { icon::CHEVRON_DOWN } else { icon::CHEVRON_RIGHT },
        FontId::new(13.0, theme::icon_family()),
        p.faint,
    );
    x += 18.0;
    if let Some(c) = dot {
        painter.text(
            egui::pos2(x + 7.0, cy),
            egui::Align2::CENTER_CENTER,
            tag_icon(label),
            FontId::new(14.0, theme::icon_family()),
            c,
        );
        x += 20.0;
    } else {
        painter.text(
            egui::pos2(x + 7.0, cy),
            egui::Align2::CENTER_CENTER,
            "\u{e4cb}",
            FontId::new(15.0, theme::icon_family()),
            p.muted,
        );
        x += 20.0;
    }
    let font = if strong { FontId::new(15.0, theme::bold_family()) } else { FontId::new(14.0, theme::bold_family()) };
    let g = painter.layout_no_wrap(label.to_string(), font, p.text);
    let lw = g.size().x;
    painter.galley(egui::pos2(x, cy - g.size().y / 2.0), g, p.text);
    x += lw + 10.0;
    let count = painter.layout_no_wrap(format!("{done}/{total}"), FontId::proportional(12.0), p.faint);
    let cw = count.size().x;
    painter.galley(egui::pos2(x, cy - count.size().y / 2.0), count, p.faint);
    x += cw + 10.0;
    if let Some(r) = repo {
        let g = painter.layout_no_wrap("\u{e0e9}".to_string(), FontId::new(12.0, theme::icon_family()), p.faint);
        painter.galley(egui::pos2(x, cy - g.size().y / 2.0), g, p.faint);
        let g2 = painter.layout_no_wrap(r.to_string(), FontId::proportional(12.0), p.faint);
        painter.galley(egui::pos2(x + 16.0, cy - g2.size().y / 2.0), g2, p.faint);
    }
    // mini progress bar on the right
    let bar = egui::Rect::from_min_size(egui::pos2(rect.right() - 130.0, cy - 3.0), egui::vec2(110.0, 6.0));
    painter.rect_filled(bar, 3.0, p.border);
    if total > 0 && done > 0 {
        let mut r = bar;
        r.set_width(bar.width() * done as f32 / total as f32);
        painter.rect_filled(r, 3.0, if done == total { Status::Done.color() } else { p.accent });
    }
    resp
}
