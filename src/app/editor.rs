//! Zone d'édition d'une note : titre intégré (renomme la note), éditeur avec live preview,
//! autocomplétion des [[liens]], cases à cocher cliquables et aperçu Markdown.

use super::layout::centered;
use super::*;

pub(super) fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

pub(super) fn editor_id(note: Uuid) -> egui::Id {
    egui::Id::unique("mnemo-editor").with(note)
}

/// Positions (byte offsets of the check char) of every task list checkbox `- [ ]` / `- [x]`.
fn task_positions(src: &str) -> Vec<(usize, bool)> {
    let mut out = vec![];
    let mut offset = 0;
    let mut fence = false;
    for line in src.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
        }
        if !fence {
            let indent = line.len() - t.len();
            let t2 = t.trim_start_matches('>').trim_start();
            let skip = line.len() - t2.len();
            let marker_len = if t2.starts_with("- ") || t2.starts_with("* ") || t2.starts_with("+ ") {
                Some(2)
            } else {
                let digits = t2.chars().take_while(char::is_ascii_digit).count();
                (digits > 0 && (t2[digits..].starts_with(". ") || t2[digits..].starts_with(") "))).then_some(digits + 2)
            };
            if let Some(m) = marker_len {
                let rest = &t2[m..];
                if rest.len() >= 3 && rest.as_bytes()[0] == b'[' && rest.as_bytes()[2] == b']' {
                    let c = rest.as_bytes()[1];
                    if c == b' ' || c == b'x' || c == b'X' {
                        out.push((offset + skip + m + 1, c != b' '));
                    }
                }
            }
            let _ = indent;
        }
        offset += line.len();
    }
    out
}

impl MnemoApp {
    pub(super) fn inline_title(&mut self, ui: &mut egui::Ui, editable: bool) {
        let p = self.palette();
        let Some(n) = &self.open else { return };
        let (id, path) = (n.id, n.path.clone());
        let base = basename(&path).to_string();
        let font = FontId::new((self.text_size() * self.metrics().h_scale[0]).round(), theme::bold_family());
        if !editable {
            ui.add(egui::Label::new(RichText::new(&base).font(font).color(p.text)).wrap());
            return;
        }
        let title_id = egui::Id::unique(("inline-title", id));
        let focused = ui.memory(|m| m.has_focus(title_id));
        let fresh = !matches!(&self.title_edit, Some((tid, orig, _)) if *tid == id && *orig == base);
        if fresh && !focused {
            self.title_edit = Some((id, base.clone(), base.clone()));
        }
        // Enter commits the title (the field is multiline only so that long titles wrap)
        if focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
            self.title_enter = true;
            ui.memory_mut(|m| m.surrender_focus(title_id));
        }
        let Some((_, _, buf)) = self.title_edit.as_mut() else { return };
        if buf.contains('\n') {
            *buf = buf.replace(['\n', '\r'], " ");
        }
        let r = ui.add(
            egui::TextEdit::multiline(buf)
                .desired_rows(1)
                .id(title_id)
                .font(font)
                .text_color(p.text)
                .frame(egui::Frame::NONE)
                .margin(egui::Margin::ZERO)
                .desired_width(f32::INFINITY)
                .hint_text("Sans titre"),
        );
        if r.lost_focus() {
            let new = buf.trim().to_string();
            let enter = std::mem::take(&mut self.title_enter);
            if !new.is_empty() && new != base {
                // remember the new name right away: a second lost_focus must not resend the rename
                if let Some((_, orig, _)) = self.title_edit.as_mut() {
                    *orig = new.clone();
                }
                let target = match parent(&path) {
                    Some(par) => format!("{par}/{new}"),
                    None => new,
                };
                match normalize_path(&target) {
                    Ok(np) => {
                        self.flush_save();
                        self.db.send(Cmd::Rename { id, new_path: np });
                    }
                    Err(e) => {
                        self.title_edit = None;
                        let now = ui.input(|i| i.time);
                        self.notice(e, true, now);
                    }
                }
            } else {
                *buf = base;
            }
            if enter {
                ui.memory_mut(|m| m.request_focus(editor_id(id)));
            }
        }
    }

    pub(super) fn editor_area(&mut self, ui: &mut egui::Ui, now: f64) {
        self.tab_bar(ui);
        if self.tabs[self.active_tab].dashboard {
            self.dashboard_ui(ui, now);
            return;
        }
        if self.open.is_none() {
            self.empty_tab(ui);
            return;
        }
        let note_id = self.open.as_ref().unwrap().id;
        self.view_header(ui);

        // ---- conflict banner
        if let Some(c) = self.open.as_mut().and_then(|n| n.conflict.as_mut()) {
            let (total, left) = merge::conflict_count(&c.blocks);
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(0xe9, 0x97, 0x3f, 36))
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.label(theme::icon_text(icon::ALERT, 15.0).color(Color32::from_rgb(0xe9, 0x97, 0x3f)));
                        ui.label(format!("Conflit avec une modification distante : {left} / {total} bloc(s) à résoudre. L'édition est en pause."));
                        if ui.button("Résoudre le conflit…").clicked() {
                            c.window_open = true;
                        }
                    });
                });
        }

        // ---- preview source cache
        if self.preview_gen != self.content_gen && self.cfg.view_mode != ViewMode::Edit {
            let (id, path, content) = self.note_ctx().unwrap();
            let pre = self.plugins.preprocess(content.clone(), Some(&NoteCtx { id, path: &path, content: &content }));
            self.preview_src = markdown::preprocess(&pre);
            self.preview_gen = self.content_gen;
        }

        let width = self.readable_width();
        match self.cfg.view_mode {
            ViewMode::Edit | ViewMode::Preview => {
                let reading = self.cfg.view_mode == ViewMode::Preview;
                egui::ScrollArea::vertical().id_salt(("page", reading)).auto_shrink([false, false]).show(ui, |ui| {
                    centered(ui, width, |ui| {
                        ui.add_space(26.0);
                        self.inline_title(ui, !reading);
                        ui.add_space(14.0);
                        if reading {
                            self.preview(ui, now);
                        } else {
                            self.editor(ui, note_id, now);
                        }
                        ui.add_space(120.0);
                    });
                });
            }
            ViewMode::Split => {
                ui.columns(2, |cols| {
                    egui::ScrollArea::vertical().id_salt("split-edit").auto_shrink([false, false]).show(
                        &mut cols[0],
                        |ui| {
                            centered(ui, width, |ui| {
                                ui.add_space(26.0);
                                self.inline_title(ui, true);
                                ui.add_space(14.0);
                                self.editor(ui, note_id, now);
                                ui.add_space(120.0);
                            });
                        },
                    );
                    egui::ScrollArea::vertical().id_salt("split-read").auto_shrink([false, false]).show(
                        &mut cols[1],
                        |ui| {
                            centered(ui, width, |ui| {
                                ui.add_space(26.0);
                                self.inline_title(ui, false);
                                ui.add_space(14.0);
                                self.preview(ui, now);
                                ui.add_space(120.0);
                            });
                        },
                    );
                });
            }
        }
    }

    pub(super) fn editor(&mut self, ui: &mut egui::Ui, note_id: Uuid, now: f64) {
        let id = editor_id(note_id);
        // Autocomplete keyboard handling must run before the TextEdit consumes the keys
        let mut accept: Option<String> = None;
        if let Some(ac) = &mut self.autocomplete
            && !ac.matches.is_empty()
            && ui.memory(|m| m.has_focus(id))
        {
            ui.input_mut(|i| {
                if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                    ac.sel = (ac.sel + 1) % ac.matches.len();
                }
                if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                    ac.sel = (ac.sel + ac.matches.len() - 1) % ac.matches.len();
                }
                if i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab) {
                    accept = Some(ac.matches[ac.sel].clone());
                }
                if i.consume_key(Modifiers::NONE, Key::Escape) {
                    self.ac_dismissed = Some(ac.start);
                }
            });
        }
        if let Some(name) = accept {
            self.accept_autocomplete(ui.ctx(), &name, now);
        }

        let pal = self.palette();
        let fs = self.text_size();
        // live preview: reveal markdown syntax only on the lines holding the cursor
        let active = if ui.memory(|m| m.has_focus(id)) {
            let content = &self.open.as_ref().unwrap().content;
            egui::TextEdit::load_state(ui.ctx(), id).and_then(|s| s.cursor.char_range()).map(|r| {
                let (a, b) = (r.primary.index.0.min(r.secondary.index.0), r.primary.index.0.max(r.secondary.index.0));
                let line_of = |ci: usize| content.chars().take(ci).filter(|c| *c == '\n').count();
                (line_of(a), line_of(b))
            })
        } else {
            None
        };
        let met = self.metrics();
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let job = theme::markdown_job(buf.as_str(), &pal, &met, fs, wrap, active);
            ui.ctx().fonts_mut(|f| f.layout_job(job))
        };
        let bg_slot = ui.painter().add(egui::Shape::Noop);
        let n = self.open.as_mut().unwrap();
        let editable = n.conflict.is_none();
        let out = egui::TextEdit::multiline(&mut n.content)
            .interactive(editable)
            .id(id)
            .layouter(&mut layouter)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .desired_width(f32::INFINITY)
            .desired_rows(20)
            .lock_focus(true)
            .hint_text(RichText::new("Commencez à écrire…").color(pal.faint))
            .show(ui);
        if out.response.changed() {
            let n = self.open.as_mut().unwrap();
            n.last_edit = Some(now);
            self.content_gen += 1;
        }
        // live-preview decorations (bullets, checkboxes, quote bars, code backgrounds)
        let boxes = {
            let n = self.open.as_ref().unwrap();
            let right = out.response.rect.right();
            theme::decorate_editor(
                &n.content,
                active,
                &out.galley,
                out.galley_pos,
                right,
                &pal,
                &met,
                ui.painter(),
                bg_slot,
            )
        };
        if out.response.clicked()
            && !ui.input(|i| i.modifiers.command)
            && let Some(pointer) = out.response.interact_pointer_pos()
            && let Some((_, b)) = boxes.iter().find(|(r, _)| r.expand(3.0).contains(pointer))
        {
            let n = self.open.as_mut().unwrap();
            if n.conflict.is_none() && n.content.is_char_boundary(*b) && n.content.is_char_boundary(b + 1) {
                let checked = &n.content[*b..b + 1] != " ";
                n.content.replace_range(*b..b + 1, if checked { " " } else { "x" });
                n.last_edit = Some(now);
                self.content_gen += 1;
            }
        }
        // Ctrl/Cmd + click on a [[link]] opens it
        // (uses the character under the pointer in *this* frame's layout: the cursor position is
        // unreliable here because focusing the editor reveals the markdown syntax and shifts the text)
        if ui.input(|i| i.pointer.primary_clicked() && i.modifiers.command)
            && out.response.hovered()
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let ci = out.galley.cursor_from_pos(pointer - out.galley_pos).index.0;
            let n = self.open.as_ref().unwrap();
            let b = char_to_byte(&n.content, ci);
            let line_start = n.content[..b].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = n.content[b..].find('\n').map(|i| b + i).unwrap_or(n.content.len());
            let line = &n.content[line_start..line_end];
            let col = b - line_start;
            if let Some(open) = line[..col.min(line.len())].rfind("[[")
                && let Some(close) = line[open..].find("]]")
                && open + close + 2 >= col
            {
                let (target, _) = markdown::parse_wikilink(&line[open + 2..open + close]);
                self.actions.push(UiAction::OpenLink(target));
            }
        }

        // ---- wikilink autocomplete
        self.autocomplete = None;
        if out.response.has_focus()
            && let Some(range) = out.cursor_range
            && range.primary == range.secondary
        {
            let n = self.open.as_ref().unwrap();
            let ci = range.primary.index.0;
            let before = &n.content[..char_to_byte(&n.content, ci)];
            if let Some(partial) = markdown::pending_wikilink(before) {
                let start = ci - partial.chars().count();
                if self.ac_dismissed != Some(start) {
                    let q = partial.to_lowercase();
                    let mut matches: Vec<String> = self
                        .notes
                        .iter()
                        .filter(|m| m.id != n.id && m.path.to_lowercase().contains(&q))
                        .map(|m| m.path.clone())
                        .collect();
                    matches.sort_by_key(|p| (!basename(p).to_lowercase().starts_with(&q), p.len()));
                    matches.truncate(8);
                    let pos = out.galley_pos + out.galley.pos_from_cursor(range.primary).left_bottom().to_vec2();
                    let show_create =
                        !partial.trim().is_empty() && !matches.iter().any(|m| m.eq_ignore_ascii_case(partial.trim()));
                    if show_create {
                        matches.push(partial.trim().to_string());
                    }
                    let prev_sel = self.prev_ac_sel.unwrap_or(0);
                    let mut ac = Autocomplete { start, partial: partial.to_string(), sel: 0, matches };
                    ac.sel = prev_sel.min(ac.matches.len().saturating_sub(1));
                    if !ac.matches.is_empty() {
                        let mut clicked = None;
                        egui::Area::new(egui::Id::unique("wikilink-ac"))
                            .order(egui::Order::Foreground)
                            .fixed_pos(pos + egui::vec2(0.0, 6.0))
                            .show(ui.ctx(), |ui| {
                                egui::Frame::popup(ui.style()).fill(pal.bg2).inner_margin(6.0).show(ui, |ui| {
                                    ui.set_width(320.0);
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    for (i, m) in ac.matches.iter().enumerate() {
                                        let (title, detail) = if show_create && i == ac.matches.len() - 1 {
                                            (m.clone(), "Créer la note".to_string())
                                        } else {
                                            (basename(m).to_string(), parent(m).unwrap_or("").to_string())
                                        };
                                        if theme::suggestion_row(ui, &title, &detail, i == ac.sel).clicked() {
                                            clicked = Some(m.clone());
                                        }
                                    }
                                    ui.add_space(4.0);
                                    ui.label(
                                        RichText::new("↑↓ naviguer   ↵ insérer   échap ignorer")
                                            .size(11.5)
                                            .color(pal.faint),
                                    );
                                });
                            });
                        if let Some(c) = clicked {
                            self.accept_autocomplete(ui.ctx(), &c, now);
                        } else {
                            self.autocomplete = Some(ac);
                        }
                    }
                }
            } else {
                self.ac_dismissed = None;
            }
        }
        self.prev_ac_sel = self.autocomplete.as_ref().map(|a| a.sel);
    }

    pub(super) fn accept_autocomplete(&mut self, ctx: &egui::Context, target: &str, now: f64) {
        let Some(ac) = self.autocomplete.take() else { return };
        let Some(n) = &mut self.open else { return };
        // Prefer the short name when it is unambiguous
        let base = basename(target);
        let ambiguous = self.notes.iter().filter(|m| basename(&m.path).eq_ignore_ascii_case(base)).count() > 1;
        let name = if ambiguous { target } else { base };
        let start_b = char_to_byte(&n.content, ac.start);
        let end_b = char_to_byte(&n.content, ac.start + ac.partial.chars().count());
        let closing = if n.content[end_b..].starts_with("]]") { "" } else { "]]" };
        n.content.replace_range(start_b..end_b, &format!("{name}{closing}"));
        let pos = ac.start + name.chars().count() + 2;
        let id = editor_id(n.id);
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(pos))));
        state.store(ctx, id);
        n.last_edit = Some(now);
        self.content_gen += 1;
        self.prev_ac_sel = None;
    }

    pub(super) fn preview(&mut self, ui: &mut egui::Ui, now: f64) {
        let fs = self.text_size();
        ui.scope(|ui| {
            {
                use egui::TextStyle::*;
                let st = ui.style_mut();
                st.text_styles.insert(Body, FontId::proportional(fs));
                st.text_styles.insert(Monospace, FontId::monospace(fs - 1.5));
                st.text_styles.insert(Heading, FontId::new(fs * 1.802, theme::bold_family()));
                st.spacing.item_spacing.y = 8.0;
            }
            let mut copy = self.preview_src.clone();
            CommonMarkViewer::new().default_width(Some(ui.available_width() as usize)).show_mut(
                ui,
                &mut self.md_cache,
                &mut copy,
            );
            if copy != self.preview_src {
                // a checkbox was toggled in the preview: apply it to the real source
                let before = task_positions(&self.preview_src);
                let after = task_positions(&copy);
                if let Some(k) = before.iter().zip(after.iter()).position(|(a, b)| a.1 != b.1)
                    && let Some(n) = &mut self.open
                {
                    let tasks = task_positions(&n.content);
                    if let Some(&(pos, checked)) = tasks.get(k) {
                        n.content.replace_range(pos..pos + 1, if checked { " " } else { "x" });
                        n.last_edit = Some(now);
                        self.content_gen += 1;
                    }
                }
            }
        });
    }
}
