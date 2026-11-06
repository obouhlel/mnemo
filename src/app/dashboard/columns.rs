//! Vue « colonnes » : tableau kanban, une colonne par statut.

use super::*;

impl MnemoApp {
    pub(super) fn columns_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<DashAction>) {
        let p = self.palette();
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
                                        acts.push(DashAction::Create {
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
                                        acts.push(DashAction::Select(if selected { None } else { Some(t.id) }));
                                    }
                                    ui.add_space(6.0);
                                }
                            },
                        );
                    });
                    if let Some(id) = dropped {
                        acts.push(DashAction::Patch(*id, TicketPatch { status: Some(*st), ..Default::default() }));
                    }
                }
            });
        });
    }
}
