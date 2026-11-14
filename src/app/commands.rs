//! Commandes de l'application (palette Ctrl+P), raccourcis clavier, traitement des actions
//! de l'interface (UiAction) et des actions demandées par les plugins.

use super::*;

#[derive(Clone, Copy, PartialEq)]
pub(super) enum CoreCmd {
    NewNote,
    NewFolder,
    Switcher,
    Palette,
    Search,
    Graph,
    NewWhiteboard,
    ToggleMode,
    SplitMode,
    RenameCurrent,
    DeleteCurrent,
    Save,
    Settings,
    ReloadPlugins,
    ToggleTheme,
    Refresh,
    ToggleLeft,
    ToggleRight,
    NewTab,
    CloseTab,
    Dashboard,
    CollapseAll,
}

pub(super) const CORE_COMMANDS: &[(CoreCmd, &str, &str)] = &[
    (CoreCmd::NewNote, "Nouvelle note", "Ctrl+N"),
    (CoreCmd::NewFolder, "Nouveau dossier", ""),
    (CoreCmd::Switcher, "Ouvrir une note…", "Ctrl+O"),
    (CoreCmd::Search, "Rechercher dans toutes les notes", "Ctrl+Shift+F"),
    (CoreCmd::Graph, "Afficher le graphe", "Ctrl+G"),
    (CoreCmd::NewWhiteboard, "Nouveau whiteboard (dessin libre)", ""),
    (CoreCmd::ToggleMode, "Basculer édition / lecture", "Ctrl+E"),
    (CoreCmd::SplitMode, "Mode divisé (édition + aperçu)", ""),
    (CoreCmd::RenameCurrent, "Renommer / déplacer la note", "F2"),
    (CoreCmd::DeleteCurrent, "Supprimer la note", ""),
    (CoreCmd::Save, "Enregistrer maintenant", "Ctrl+S"),
    (CoreCmd::Settings, "Paramètres", "Ctrl+,"),
    (CoreCmd::ReloadPlugins, "Recharger les plugins", ""),
    (CoreCmd::ToggleTheme, "Basculer thème clair / sombre", ""),
    (CoreCmd::Refresh, "Resynchroniser avec la base", ""),
    (CoreCmd::Dashboard, "Ouvrir le tableau de bord (projets et tickets)", "Ctrl+Shift+D"),
    (CoreCmd::NewTab, "Nouvel onglet", "Ctrl+T"),
    (CoreCmd::CloseTab, "Fermer l'onglet", "Ctrl+W"),
    (CoreCmd::CollapseAll, "Replier tous les dossiers", ""),
    (CoreCmd::ToggleLeft, "Afficher / masquer l'explorateur", "Ctrl+Shift+L"),
    (CoreCmd::ToggleRight, "Afficher / masquer le panneau de droite", "Ctrl+Shift+R"),
];

impl MnemoApp {
    pub(super) fn process_actions(&mut self, ctx: &egui::Context, now: f64) {
        for a in std::mem::take(&mut self.actions) {
            match a {
                UiAction::OpenId(id) => self.open_id(id),
                UiAction::OpenLink(l) => {
                    if let Some(rest) = l.strip_prefix("ghost:") {
                        self.flush_save();
                        self.db.send(Cmd::OpenLink(rest.to_string()));
                    } else if let Ok(id) = Uuid::parse_str(&l) {
                        self.open_id(id);
                    } else {
                        self.flush_save();
                        self.db.send(Cmd::OpenLink(l));
                    }
                }
                UiAction::OpenTag(t) => {
                    self.left_tab = LeftTab::Search;
                    self.search_q = format!("#{t}");
                    self.search_due = Some(now);
                }
                UiAction::NewNoteIn(folder) => {
                    self.dialog = Some(Dialog::NewNote { input: format!("{folder}/"), error: None });
                    self.focus_dialog = true;
                }
                UiAction::NewFolderIn(folder) => {
                    self.dialog = Some(Dialog::NewFolder { input: format!("{folder}/"), error: None });
                    self.focus_dialog = true;
                }
                UiAction::RenameNote(id, path) => {
                    self.dialog = Some(Dialog::RenameNote { id, input: path, error: None });
                    self.focus_dialog = true;
                }
                UiAction::RenameFolder(p) => {
                    self.dialog = Some(Dialog::RenameFolder { from: p.clone(), input: p, error: None });
                    self.focus_dialog = true;
                }
                UiAction::DeleteNote(id, path) => self.dialog = Some(Dialog::DeleteNote { id, path }),
                UiAction::DeleteFolder(path) => self.dialog = Some(Dialog::DeleteFolder { path }),
                UiAction::Plugin(i, id) => {
                    let nc = self.note_ctx();
                    let note = nc.as_ref().map(|(id, path, content)| NoteCtx { id: id.clone(), path, content });
                    self.plugins.run_command(i, &id, note.as_ref());
                    self.apply_plugin_actions(ctx, now);
                }
                UiAction::Core(c) => self.run_core(c, ctx, now),
                UiAction::OpenNewTab(id) => self.open_in(id, OpenMode::NewTab),
                UiAction::SwitchTab(i) => {
                    if i != self.active_tab {
                        self.activate_tab(i);
                    }
                }
                UiAction::CloseTab(i) => self.close_tab(i),
                UiAction::CloseOtherTabs(i) => {
                    let keep = self.tabs[i].clone();
                    self.tabs = vec![keep];
                    self.activate_tab(0);
                }
                UiAction::ToggleFolder(p) => {
                    let k = p.to_lowercase();
                    if !self.expanded.remove(&k) {
                        self.expanded.insert(k);
                    }
                }
                UiAction::SetSort(o) => self.sort = o,
                UiAction::Back => self.navigate(false),
                UiAction::Forward => self.navigate(true),
                UiAction::Move(item, target) => self.move_item(item, &target, now),
                UiAction::InsertEmbed(rel) => {
                    let text = self.embed_text(&rel);
                    self.insert_at_cursor(ctx, &text, now);
                }
                UiAction::NewWhiteboardIn(folder) => self.new_whiteboard(&folder),
                UiAction::OpenFile(rel) => {
                    if rel.to_lowercase().ends_with(".mnemo-board") {
                        self.open_whiteboard(&rel);
                    } else if crate::markdown::is_image(&rel) {
                        self.image_view = Some(rel);
                    } else if let Some(abs) = self.file_abs(&rel) {
                        ctx.open_url(egui::OpenUrl::new_tab(format!("file://{}", abs.display())));
                    }
                }
                UiAction::DeleteFile(rel) => self.db.send(Cmd::DeleteFile(rel)),
                UiAction::PickIcon(path) => {
                    self.icon_pick = Some(icons::IconPick { path, query: String::new(), focus: true })
                }
                UiAction::ImportFiles(files, target) => self.import_dropped(files, &target, now),
            }
        }
    }

    pub(super) fn run_core(&mut self, c: CoreCmd, ctx: &egui::Context, now: f64) {
        let current_folder =
            || self.open.as_ref().and_then(|n| parent(&n.path).map(|p| format!("{p}/"))).unwrap_or_default();
        match c {
            CoreCmd::NewNote => {
                self.dialog = Some(Dialog::NewNote { input: current_folder(), error: None });
                self.focus_dialog = true;
            }
            CoreCmd::NewFolder => {
                self.dialog = Some(Dialog::NewFolder { input: current_folder(), error: None });
                self.focus_dialog = true;
            }
            CoreCmd::Switcher => {
                self.dialog = Some(Dialog::Switcher { query: String::new(), sel: 0 });
                self.focus_dialog = true;
            }
            CoreCmd::Palette => {
                self.dialog = Some(Dialog::Palette { query: String::new(), sel: 0 });
                self.focus_dialog = true;
            }
            CoreCmd::Search => {
                self.left_tab = LeftTab::Search;
                self.focus_search = true;
                if !self.cfg.show_left {
                    self.cfg.show_left = true;
                    self.cfg.save();
                }
            }
            CoreCmd::Graph => self.open_graph(),
            CoreCmd::NewWhiteboard => {
                // dans le dossier de la note ouverte, sinon à la racine
                let folder = self.open.as_ref().and_then(|n| parent(&n.path).map(str::to_string)).unwrap_or_default();
                self.new_whiteboard(&folder);
            }
            CoreCmd::ToggleMode => {
                self.cfg.view_mode =
                    if self.cfg.view_mode == ViewMode::Preview { ViewMode::Edit } else { ViewMode::Preview };
                self.cfg.save();
            }
            CoreCmd::SplitMode => {
                self.cfg.view_mode = ViewMode::Split;
                self.cfg.save();
            }
            CoreCmd::RenameCurrent => {
                if let Some(n) = &self.open {
                    self.actions.push(UiAction::RenameNote(n.id, n.path.clone()));
                    self.process_actions(ctx, now);
                }
            }
            CoreCmd::DeleteCurrent => {
                if let Some(n) = &self.open {
                    self.dialog = Some(Dialog::DeleteNote { id: n.id, path: n.path.clone() });
                }
            }
            CoreCmd::Save => {
                self.flush_save();
            }
            CoreCmd::Settings => self.show_settings = !self.show_settings,
            CoreCmd::ReloadPlugins => {
                self.plugins.reload();
                self.plugin_changed();
                self.notice(format!("{} plugin(s) chargé(s)", self.plugins.plugins.len()), false, now);
            }
            CoreCmd::ToggleTheme => {
                self.cfg.dark_mode = !self.cfg.dark_mode;
                self.cfg.save();
            }
            CoreCmd::NewTab => {
                self.leave_note();
                self.tabs.push(Tab::default());
                self.active_tab = self.tabs.len() - 1;
                self.open = None;
            }
            CoreCmd::CloseTab => self.close_tab(self.active_tab),
            CoreCmd::Dashboard => self.open_dashboard(None),
            CoreCmd::CollapseAll => self.expanded.clear(),
            CoreCmd::ToggleLeft => {
                self.cfg.show_left = !self.left_visible(ctx);
                self.cfg.save();
            }
            CoreCmd::ToggleRight => {
                self.cfg.show_right = !self.right_visible(ctx);
                self.cfg.save();
            }
            CoreCmd::Refresh => {
                self.need = Needs {
                    notes: true,
                    folders: true,
                    plugins: true,
                    backlinks: true,
                    tags: true,
                    graph: self.graph_shown(),
                    board: true,
                    icons: true,
                    files: true,
                };
                if let Some(n) = &self.open {
                    self.db.send(Cmd::Load { id: n.id, purpose: LoadPurpose::Refresh });
                }
            }
        }
    }

    pub(super) fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = Modifiers::COMMAND;
        let table: &[(KeyboardShortcut, CoreCmd)] = &[
            (KeyboardShortcut::new(cmd, Key::P), CoreCmd::Palette),
            (KeyboardShortcut::new(cmd, Key::O), CoreCmd::Switcher),
            (KeyboardShortcut::new(cmd, Key::N), CoreCmd::NewNote),
            (KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::F), CoreCmd::Search),
            (KeyboardShortcut::new(cmd, Key::G), CoreCmd::Graph),
            (KeyboardShortcut::new(cmd, Key::E), CoreCmd::ToggleMode),
            (KeyboardShortcut::new(cmd, Key::S), CoreCmd::Save),
            (KeyboardShortcut::new(cmd, Key::Comma), CoreCmd::Settings),
            (KeyboardShortcut::new(Modifiers::NONE, Key::F2), CoreCmd::RenameCurrent),
            (KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::L), CoreCmd::ToggleLeft),
            (KeyboardShortcut::new(cmd, Key::T), CoreCmd::NewTab),
            (KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::D), CoreCmd::Dashboard),
            (KeyboardShortcut::new(cmd, Key::W), CoreCmd::CloseTab),
            (KeyboardShortcut::new(cmd | Modifiers::SHIFT, Key::R), CoreCmd::ToggleRight),
        ];
        if self.dialog.is_some() {
            return;
        }
        for (sc, c) in table {
            if ctx.input_mut(|i| i.consume_shortcut(sc)) {
                self.actions.push(UiAction::Core(*c));
            }
        }
    }

    pub(super) fn apply_plugin_actions(&mut self, ctx: &egui::Context, now: f64) {
        for a in self.plugins.take_actions() {
            match a {
                Action::Notice(m) => self.notice(m, false, now),
                Action::OpenNote(p) => self.db.send(Cmd::OpenLink(p)),
                Action::CreateNote { path, content } => match normalize_path(&path) {
                    Ok(path) => self.db.send(Cmd::Create { path, content, open: true, if_missing: true }),
                    Err(e) => self.notice(e, true, now),
                },
                Action::InsertText(t) => self.insert_at_cursor(ctx, &t, now),
                Action::ReplaceContent(t) => {
                    if let Some(n) = &mut self.open {
                        n.content = t;
                        n.last_edit = Some(now);
                        self.content_gen += 1;
                    }
                }
            }
        }
    }

    pub(super) fn insert_at_cursor(&mut self, ctx: &egui::Context, text: &str, now: f64) {
        let Some(n) = &mut self.open else { return };
        let id = editor_id(n.id);
        let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
        let total = n.content.chars().count();
        let (a, b) = state
            .cursor
            .char_range()
            .map(|r| {
                let (x, y) = (r.primary.index.0, r.secondary.index.0);
                (x.min(y).min(total), x.max(y).min(total))
            })
            .unwrap_or((total, total));
        let (ba, bb) = (char_to_byte(&n.content, a), char_to_byte(&n.content, b));
        n.content.replace_range(ba..bb, text);
        let pos = a + text.chars().count();
        state.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(pos))));
        state.store(ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        n.last_edit = Some(now);
        self.content_gen += 1;
    }
}
