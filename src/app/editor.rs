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
    /// Dessin d'un whiteboard pour un aperçu : celui ouvert dans un onglet, sinon le fichier (mis en cache).
    pub(super) fn board_sketch(&mut self, rel: &str) -> Option<crate::sketch::Sketch> {
        if let Some(b) = self.boards.get(rel) {
            return Some(b.sketch.clone());
        }
        let abs = self.file_abs(rel)?;
        let m = std::fs::metadata(&abs).and_then(|m| m.modified()).ok();
        if let Some((cm, sk)) = self.sketch_cache.get(rel)
            && *cm == m
        {
            return Some(sk.clone());
        }
        let sk = crate::sketch::Sketch::parse(&std::fs::read_to_string(&abs).ok()?).ok()?;
        self.sketch_cache.insert(rel.to_string(), (m, sk.clone()));
        Some(sk)
    }

    /// Lignes de la note ouverte qui n'intègrent qu'une image (`![[photo.png]]`, `![](img/a.png)`), hors des
    /// lignes du curseur : numéro de ligne → (adresse, taille d'affichage). Taille réelle de l'image, réduite
    /// à la largeur de la colonne ; pendant le chargement, une hauteur provisoire.
    pub(super) fn editor_images(
        &mut self,
        ui: &egui::Ui,
        active: Option<(usize, usize)>,
    ) -> std::collections::HashMap<usize, (String, egui::Vec2)> {
        let mut out = std::collections::HashMap::new();
        let Some(n) = self.open.as_ref().map(|n| (n.content.clone(), n.path.clone())) else { return out };
        let (content, path) = n;
        let max_w = ui.available_width().max(100.0);
        let mut fence = false;
        for (no, line) in content.lines().enumerate() {
            let t = line.trim();
            if t.starts_with("```") || t.starts_with("~~~") {
                fence = !fence;
            }
            if fence || active.is_some_and(|(a, b)| no >= a && no <= b) {
                continue;
            }
            let target = if let Some(inner) = t.strip_prefix("![[").and_then(|r| r.strip_suffix("]]")) {
                inner.split('|').next().unwrap_or("").trim().to_string()
            } else if t.starts_with("![") && t.ends_with(')') && !t[2..].contains("![") {
                match t.find("](") {
                    Some(i) => t[i + 2..t.len() - 1].trim().trim_matches(['<', '>']).replace("%20", " "),
                    None => continue,
                }
            } else {
                continue;
            };
            let uri = match self.resolve_embed(&target, &path) {
                Some(markdown::Embed::Image(uri)) => uri,
                // whiteboard : aperçu du dessin, cliquable (« board:<chemin> »)
                Some(markdown::Embed::Board(rel)) => {
                    let h = self
                        .board_sketch(&rel)
                        .and_then(|s| s.bounds())
                        .map(|b| ((b[3] - b[1]) / (b[2] - b[0]).max(1.0) * max_w.min(640.0)).clamp(140.0, 420.0))
                        .unwrap_or(140.0);
                    out.insert(no, (format!("board:{rel}"), egui::vec2(max_w.min(640.0), h)));
                    continue;
                }
                None => continue,
            };
            let size = match ui.ctx().try_load_texture(&uri, egui::TextureOptions::default(), egui::SizeHint::default())
            {
                Ok(egui::load::TexturePoll::Ready { texture }) => texture.size,
                _ => {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
                    egui::vec2(max_w.min(400.0), 160.0)
                }
            };
            let scale = (max_w / size.x).min(1.0).min(600.0 / size.y.max(1.0));
            out.insert(no, (uri, size * scale));
        }
        out
    }

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
        if self.tabs[self.active_tab].graph {
            self.graph_ui(ui);
            return;
        }
        if let Some(rel) = self.tabs[self.active_tab].board.clone() {
            self.whiteboard_ui(ui, &rel, now);
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

        // Ctrl+V avec une image dans le presse-papiers : pièce jointe intégrée au curseur
        self.paste_image(ui.ctx());

        // ---- preview source cache
        if self.preview_gen != self.content_gen && self.cfg.view_mode != ViewMode::Edit {
            let (id, path, content) = self.note_ctx().unwrap();
            let pre = self.plugins.preprocess(content.clone(), Some(&NoteCtx { id, path: &path, content: &content }));
            self.preview_src = markdown::preprocess_with(&pre, &|t| self.resolve_embed(t, &path));
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
        // images intégrées seules sur leur ligne : affichées dans l'éditeur (hors ligne du curseur)
        let images = self.editor_images(ui, active);
        let heights: std::collections::HashMap<usize, f32> = images.iter().map(|(l, (_, s))| (*l, s.y + 8.0)).collect();
        let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
            let job = theme::markdown_job(buf.as_str(), &pal, &met, fs, wrap, active, &heights);
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
        // images à la place de leur ligne
        {
            let n = self.open.as_ref().unwrap();
            let mut starts = vec![0usize];
            starts.extend(n.content.chars().enumerate().filter(|(_, c)| *c == '\n').map(|(i, _)| i + 1));
            for (line, (uri, size)) in &images {
                let Some(&ci) = starts.get(*line) else { continue };
                let row = out.galley.pos_from_cursor(egui::text::CCursor::new(ci)).translate(out.galley_pos.to_vec2());
                let rect = egui::Rect::from_min_size(egui::pos2(out.galley_pos.x, row.bottom() - size.y - 4.0), *size);
                if let Some(rel) = uri.strip_prefix("board:") {
                    ui.painter().rect(rect, 6.0, pal.bg2, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
                    if let Some(sk) = self.board_sketch(rel) {
                        super::whiteboard::paint_preview(ui.painter(), &sk, rect, &pal);
                    }
                    let hint = "Cliquer pour ouvrir le whiteboard";
                    ui.painter().text(
                        rect.right_bottom() - egui::vec2(8.0, 6.0),
                        egui::Align2::RIGHT_BOTTOM,
                        hint,
                        FontId::proportional(11.0),
                        pal.faint,
                    );
                    if out.response.clicked() && out.response.interact_pointer_pos().is_some_and(|p| rect.contains(p)) {
                        self.actions.push(UiAction::OpenFile(rel.to_string()));
                    }
                } else {
                    egui::Image::from_uri(uri.clone()).corner_radius(4.0).paint_at(ui, rect);
                }
            }
        }
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
