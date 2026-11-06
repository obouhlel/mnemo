//! Vue « lignes » : tickets regroupés par projet puis par tag, avec barres de progression.

use super::*;

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

impl MnemoApp {
    pub(super) fn rows_view(&mut self, ui: &mut egui::Ui, acts: &mut Vec<DashAction>) {
        let p = self.palette();
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
                            acts.push(DashAction::Toggle(pkey.clone()));
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
                            acts.push(DashAction::Toggle(gkey.clone()));
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
                                acts.push(DashAction::Create {
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
    pub(super) fn ticket_row(
        &self,
        ui: &mut egui::Ui,
        p: &theme::Palette,
        t: &Ticket,
        reports: usize,
        depth: usize,
        salt: &str,
        acts: &mut Vec<DashAction>,
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
            acts.push(DashAction::Patch(
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
            acts.push(DashAction::Select(if selected { None } else { Some(t.id) }));
        }
        resp.context_menu(|ui| {
            ui.label(RichText::new("Changer le statut").small().color(p.faint));
            for st in Status::ALL {
                if ui.selectable_label(t.status == st, st.label()).clicked() {
                    acts.push(DashAction::Patch(t.id, TicketPatch { status: Some(st), ..Default::default() }));
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Nouveau rapport").clicked() {
                acts.push(DashAction::NewReport(t.id));
                ui.close();
            }
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                acts.push(DashAction::Delete(t.id));
                ui.close();
            }
        });
    }
}
