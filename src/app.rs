mod appearance;
mod conflict;
mod dashboard;
mod editor;
mod settings;
mod state;

use editor::{char_to_byte, editor_id};
use state::*;

use crate::board::Board;
use crate::config::{Config, ViewMode, redact_url};
use crate::db::{Cmd, DbHandle, LoadPurpose, Msg};
use crate::graph::GraphView;
use crate::markdown;
use crate::merge;
use crate::model::*;
use crate::plugins::{Action, NoteCtx, PluginHost, toml_to_json};
use crate::theme::{self, icon};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Color32, FontId, Key, KeyboardShortcut, Modifiers, RichText, TextFormat};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::collections::HashMap;
use std::sync::mpsc::Receiver;
use uuid::Uuid;

const AUTOSAVE_DELAY: f64 = 0.6;

#[derive(Clone, Copy, PartialEq)]
enum CoreCmd {
    NewNote,
    NewFolder,
    Switcher,
    Palette,
    Search,
    Graph,
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
    DailyNote,
    Templates,
    CollapseAll,
}

const CORE_COMMANDS: &[(CoreCmd, &str, &str)] = &[
    (CoreCmd::NewNote, "Nouvelle note", "Ctrl+N"),
    (CoreCmd::NewFolder, "Nouveau dossier", ""),
    (CoreCmd::Switcher, "Ouvrir une note…", "Ctrl+O"),
    (CoreCmd::Search, "Rechercher dans toutes les notes", "Ctrl+Shift+F"),
    (CoreCmd::Graph, "Afficher le graphe", "Ctrl+G"),
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
    (CoreCmd::DailyNote, "Ouvrir la note du jour", ""),
    (CoreCmd::CollapseAll, "Replier tous les dossiers", ""),
    (CoreCmd::ToggleLeft, "Afficher / masquer l'explorateur", "Ctrl+Shift+L"),
    (CoreCmd::ToggleRight, "Afficher / masquer le panneau de droite", "Ctrl+Shift+R"),
];

pub struct MnemoApp {
    cfg: Config,
    client_id: String,
    db: DbHandle,
    rx: Receiver<Msg>,
    conn: ConnState,
    live: bool,
    url_input: String,
    show_password: bool,

    notes: Vec<NoteMeta>,
    folders: Vec<String>,
    tree: TreeFolder,
    open: Option<OpenNote>,
    restore_done: bool,

    left_tab: LeftTab,
    search_q: String,
    search_due: Option<f64>,
    search_results: Vec<SearchHit>,
    tags: Vec<(String, i64)>,
    backlinks: Vec<Backlink>,

    md_cache: CommonMarkCache,
    content_gen: u64,
    preview_gen: u64,
    preview_src: String,
    plugin_gen: u64,
    plugin_computed: (u64, u64, f64),
    plugin_status: Vec<String>,
    plugin_panels: Vec<(String, String)>,
    plugins: PluginHost,
    plugin_cmds: Vec<(usize, String, String)>,
    plugin_edit: HashMap<String, serde_json::Map<String, serde_json::Value>>,

    dialog: Option<Dialog>,
    focus_dialog: bool,
    show_settings: bool,
    settings_tab: SettingsTab,
    graph: GraphView,
    autocomplete: Option<Autocomplete>,
    prev_ac_sel: Option<usize>,
    ac_dismissed: Option<usize>,
    notices: Vec<(String, f64, bool)>,
    actions: Vec<UiAction>,
    need: Needs,
    theme_applied: Option<(bool, f32, u64)>,
    title_shown: String,
    tabs: Vec<Tab>,
    active_tab: usize,
    open_mode: OpenMode,
    expanded: std::collections::HashSet<String>,
    sort: SortOrder,
    right_tab: usize,
    /// inline title editing: (note id, basename when editing started, buffer)
    title_edit: Option<(Uuid, String, String)>,
    css: CssState,
    /// focus the search box on the next frame (a click on the tab icon would steal it now)
    focus_search: bool,
    /// Enter was pressed in the inline title: move to the editor once the rename is sent
    title_enter: bool,
    board: Board,
    dash: dashboard::DashState,
    github_input: String,
}

/// Lowercase and strip French accents, for fuzzy matching ("theme" finds "Thème").
fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' | 'á' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' | 'í' => 'i',
            'ô' | 'ö' | 'ó' => 'o',
            'ù' | 'û' | 'ü' | 'ú' => 'u',
            'ç' => 'c',
            'ÿ' => 'y',
            c => c,
        })
        .collect()
}

impl MnemoApp {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        theme::install_fonts(&cc.egui_ctx);
        let client_id = Uuid::new_v4().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        let db = crate::db::spawn(client_id.clone(), tx, move || ctx.request_repaint());
        let mut plugins = PluginHost::new(cfg.plugins_dir());
        plugins.reload();
        let url_input = cfg.database_url.clone();
        let mut app = Self {
            client_id,
            db,
            rx,
            conn: ConnState::Disconnected(None),
            live: false,
            url_input,
            show_password: false,
            notes: vec![],
            folders: vec![],
            tree: TreeFolder::default(),
            open: None,
            restore_done: false,
            left_tab: LeftTab::Files,
            search_q: String::new(),
            search_due: None,
            search_results: vec![],
            tags: vec![],
            backlinks: vec![],
            md_cache: CommonMarkCache::default(),
            content_gen: 0,
            preview_gen: u64::MAX,
            preview_src: String::new(),
            plugin_gen: 0,
            plugin_computed: (u64::MAX, u64::MAX, 0.0),
            plugin_status: vec![],
            plugin_panels: vec![],
            plugins,
            plugin_cmds: vec![],
            plugin_edit: HashMap::new(),
            dialog: None,
            focus_dialog: false,
            show_settings: false,
            settings_tab: SettingsTab::Connection,
            graph: GraphView::default(),
            autocomplete: None,
            prev_ac_sel: None,
            ac_dismissed: None,
            notices: vec![],
            actions: vec![],
            need: Needs::default(),
            theme_applied: None,
            title_shown: String::new(),
            tabs: vec![Tab::default()],
            active_tab: 0,
            open_mode: OpenMode::Replace,
            expanded: Default::default(),
            sort: SortOrder::NameAsc,
            right_tab: 0,
            title_edit: None,
            css: CssState::default(),
            focus_search: false,
            title_enter: false,
            board: Board::default(),
            dash: dashboard::DashState::default(),
            github_input: String::new(),
            cfg,
        };
        app.css.obsidian_input = app.cfg.obsidian_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        app.install_example_snippet();
        app.reload_css();
        if !app.cfg.database_url.trim().is_empty() {
            app.connect();
        }
        app
    }

    fn connect(&mut self) {
        self.cfg.database_url = self.url_input.trim().to_string();
        self.cfg.save();
        self.conn = ConnState::Connecting;
        self.db.send(Cmd::Connect(self.cfg.database_url.clone()));
    }

    fn notice(&mut self, msg: impl Into<String>, error: bool, now: f64) {
        self.notices.push((msg.into(), now + if error { 7.0 } else { 3.5 }, error));
    }

    // ------------------------------------------------------------------ messages
    fn handle_messages(&mut self, now: f64) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Connecting => self.conn = ConnState::Connecting,
                Msg::Connected => {
                    self.conn = ConnState::Connected;
                    self.restore_done = false;
                    self.need = Needs {
                        notes: true,
                        folders: true,
                        plugins: true,
                        tags: true,
                        board: true,
                        ..Default::default()
                    };
                    self.send_github_config();
                }
                Msg::ConnectionFailed(e) => self.conn = ConnState::Disconnected(Some(e)),
                Msg::Live(v) => self.live = v,
                Msg::Resync => {
                    self.need = Needs {
                        notes: true,
                        folders: true,
                        plugins: true,
                        backlinks: true,
                        tags: true,
                        graph: self.graph.open,
                        board: true,
                    };
                    if let Some(n) = &self.open {
                        self.db.send(Cmd::Load { id: n.id, purpose: LoadPurpose::Refresh });
                    }
                }
                Msg::Notes(notes) => {
                    self.notes = notes;
                    self.rebuild_tree();
                    self.plugins.set_note_paths(self.notes.iter().map(|n| n.path.clone()).collect());
                    if !self.restore_done {
                        self.restore_done = true;
                        if self.open.is_none() {
                            let target = self
                                .cfg
                                .last_note
                                .as_ref()
                                .and_then(|p| self.notes.iter().find(|n| &n.path == p))
                                .or_else(|| self.notes.iter().find(|n| n.path.eq_ignore_ascii_case("Bienvenue")))
                                .map(|n| n.id);
                            if let Some(id) = target {
                                self.open_id(id);
                            } else if self.notes.is_empty() {
                                self.db.send(Cmd::Create {
                                    path: "Bienvenue".into(),
                                    content: WELCOME.into(),
                                    open: true,
                                    if_missing: true,
                                });
                            }
                        }
                    }
                }
                Msg::Folders(f) => {
                    self.folders = f;
                    self.rebuild_tree();
                }
                Msg::Loaded { note, purpose: LoadPurpose::Open } | Msg::Created { note, open: true } => {
                    self.set_open(note, now)
                }
                Msg::Loaded { note, purpose: LoadPurpose::Refresh } => self.apply_refresh(note, now),
                Msg::Created { .. } => {}
                Msg::Saved(note) => {
                    if let Some(n) = self.open.as_mut().filter(|n| n.id == note.id) {
                        n.version = note.version;
                        if let Some(s) = n.saving.take() {
                            n.saved = s;
                        }
                        n.path = note.path.clone();
                        if n.content != n.saved {
                            n.last_edit.get_or_insert(now);
                        }
                        let (id, path, content) = (n.id.to_string(), n.path.clone(), n.content.clone());
                        self.plugins.hook("on_save", Some(&NoteCtx { id, path: &path, content: &content }));
                    }
                }
                Msg::Conflict(current) => {
                    let mut diverged = None;
                    if let Some(n) = self.open.as_mut().filter(|n| n.id == current.id) {
                        n.saving = None;
                        if current.content == n.content {
                            n.version = current.version;
                            n.saved = current.content;
                        } else {
                            diverged = Some(current);
                        }
                    }
                    if let Some(c) = diverged {
                        self.on_divergence(c, now);
                    }
                }
                Msg::Renamed { note, relinked } => {
                    if let Some(n) = self.open.as_mut().filter(|n| n.id == note.id) {
                        n.path = note.path.clone();
                        n.version = note.version;
                    }
                    self.cfg.last_note = Some(note.path.clone());
                    if relinked > 0 {
                        self.notice(format!("Renommée — liens mis à jour dans {relinked} note(s)"), false, now);
                    }
                }
                Msg::Deleted(id) => {
                    if self.open.as_ref().is_some_and(|n| n.id == id) {
                        self.open = None;
                    }
                    self.forget_note(id);
                }
                Msg::SearchResults { query, hits } => {
                    if query == self.search_q {
                        self.search_results = hits;
                    }
                }
                Msg::Backlinks { id, links } => {
                    if self.open.as_ref().is_some_and(|n| n.id == id) {
                        self.backlinks = links;
                    }
                }
                Msg::Tags(t) => self.tags = t,
                Msg::Graph(g) => self.graph.set_data(g),
                Msg::Plugins(states) => {
                    self.plugins.apply_states(states);
                    self.plugin_changed();
                }
                Msg::Event(ev) => self.handle_event(ev, now),
                Msg::Board(b) => self.board = b,
                Msg::TicketCreated(_) => {} // keep the focus in the inline input for fast entry
                Msg::GithubDone(project, msg) => {
                    if let Some(pid) = project {
                        self.dash.syncing.remove(&pid);
                    }
                    self.dash.publishing.clear();
                    self.notice(msg, false, now);
                }
                Msg::GithubFailed(project, msg) => {
                    if let Some(pid) = project {
                        self.dash.syncing.remove(&pid);
                    }
                    self.dash.publishing.clear();
                    self.notice(format!("GitHub : {msg}"), true, now);
                }
                Msg::GithubUser(r) => self.dash.github_user = Some(r),
                Msg::Error(e) => self.notice(e, true, now),
            }
        }
    }

    fn handle_event(&mut self, ev: VaultEvent, now: f64) {
        self.plugins.on_event(&ev);
        match ev.kind.as_str() {
            "note" => {
                self.need.notes = true;
                self.need.board = true; // report paths
                self.need.backlinks = true;
                self.need.tags = true;
                self.need.graph |= self.graph.open;
                if !self.search_q.trim().is_empty() {
                    self.search_due.get_or_insert(now + 0.3);
                }
                let mine = ev.updated_by.as_deref() == Some(self.client_id.as_str());
                if let Some(n) = &self.open
                    && Some(n.id) == ev.note_id()
                {
                    if ev.op == "delete" {
                        if !mine {
                            let p = n.path.clone();
                            let id = n.id;
                            self.open = None;
                            self.forget_note(id);
                            self.notice(format!("« {p} » a été supprimée depuis un autre appareil"), true, now);
                        }
                    } else if ev.version.is_some_and(|v| v > n.version) {
                        self.db.send(Cmd::Load { id: n.id, purpose: LoadPurpose::Refresh });
                    }
                }
            }
            "folder" => self.need.folders = true,
            "board" => self.need.board = true,
            "plugin" => self.need.plugins = true,
            _ => {}
        }
    }

    fn plugin_changed(&mut self) {
        self.plugin_cmds = self.plugins.commands();
        self.plugin_gen += 1;
        self.preview_gen = u64::MAX;
    }

    fn flush_needs(&mut self) {
        if self.conn != ConnState::Connected {
            return;
        }
        let n = std::mem::take(&mut self.need);
        if n.notes {
            self.db.send(Cmd::ListNotes);
        }
        if n.folders {
            self.db.send(Cmd::ListFolders);
        }
        if n.board {
            self.db.send(Cmd::LoadBoard);
        }
        if n.plugins {
            self.db.send(Cmd::LoadPlugins);
        }
        if n.tags {
            self.db.send(Cmd::Tags);
        }
        if n.graph {
            self.db.send(Cmd::Graph);
        }
        if n.backlinks
            && let Some(o) = &self.open
        {
            self.db.send(Cmd::Backlinks(o.id));
        }
    }

    fn set_open(&mut self, note: Note, now: f64) {
        self.flush_save();
        let mut mode = std::mem::replace(&mut self.open_mode, OpenMode::Replace);
        if mode == OpenMode::Replace && self.tabs[self.active_tab].dashboard {
            mode = OpenMode::NewTab;
        }
        match mode {
            OpenMode::NewTab => {
                self.tabs.push(Tab { note: Some(note.id), ..Default::default() });
                self.active_tab = self.tabs.len() - 1;
            }
            OpenMode::Replace => {
                let t = &mut self.tabs[self.active_tab];
                if let Some(prev) = t.note
                    && prev != note.id
                {
                    t.back.push(prev);
                    t.fwd.clear();
                }
                t.note = Some(note.id);
            }
            OpenMode::History => self.tabs[self.active_tab].note = Some(note.id),
        }
        // reveal the note in the file explorer
        let mut acc = String::new();
        for seg in note.path.split('/').collect::<Vec<_>>().iter().rev().skip(1).rev() {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            self.expanded.insert(acc.to_lowercase());
        }
        self.cfg.last_note = Some(note.path.clone());
        self.cfg.save();
        self.db.send(Cmd::Backlinks(note.id));
        self.backlinks.clear();
        self.autocomplete = None;
        self.open = Some(OpenNote {
            id: note.id,
            path: note.path.clone(),
            saved: note.content.clone(),
            content: note.content,
            version: note.version,
            last_edit: None,
            saving: None,
            conflict: None,
            remote_flash: 0.0,
        });
        self.content_gen += 1;
        let n = self.open.as_ref().unwrap();
        let (id, path, content) = (n.id.to_string(), n.path.clone(), n.content.clone());
        self.plugins.hook("on_open", Some(&NoteCtx { id, path: &path, content: &content }));
        let _ = now;
    }

    /// A newer version of the open note arrived from the database (another device / window / SQL).
    fn apply_refresh(&mut self, note: Note, now: f64) {
        let Some(n) = self.open.as_mut().filter(|n| n.id == note.id) else { return };
        if note.version <= n.version {
            return;
        }
        n.path = note.path.clone();
        if !n.dirty() {
            n.content = note.content.clone();
            n.saved = note.content;
            n.version = note.version;
            n.remote_flash = now;
            self.content_gen += 1;
        } else if note.content != n.content {
            if n.saving.is_some() {
                return; // our pending save will be rejected and report the conflict itself
            }
            self.on_divergence(note, now);
        } else {
            n.version = note.version;
            n.saved = note.content;
        }
    }

    /// Local edits and a newer remote version diverged: merge automatically when the
    /// edits don't overlap, otherwise open the GitHub-like resolver.
    fn on_divergence(&mut self, remote: Note, now: f64) {
        let Some(n) = self.open.as_mut().filter(|n| n.id == remote.id) else { return };
        let blocks = merge::build(&n.saved, &n.content, &remote.content);
        let (total, _) = merge::conflict_count(&blocks);
        if total == 0 {
            n.content = merge::assemble(&blocks).unwrap_or_else(|| n.content.clone());
            n.saved = remote.content;
            n.version = remote.version;
            n.conflict = None;
            n.remote_flash = now;
            n.last_edit = Some(now - AUTOSAVE_DELAY);
            self.content_gen += 1;
            self.notice("Modifications distantes fusionnées automatiquement avec les vôtres", false, now);
        } else {
            n.conflict = Some(Conflict {
                remote,
                blocks,
                tab: ConflictTab::Resolve,
                diff_against_result: false,
                window_open: true,
            });
            self.notice(format!("Conflit : {total} bloc(s) modifié(s) des deux côtés à résoudre"), true, now);
        }
    }

    fn open_id(&mut self, id: Uuid) {
        self.open_in(id, OpenMode::Replace);
    }

    fn open_in(&mut self, id: Uuid, mode: OpenMode) {
        if mode != OpenMode::NewTab && self.open.as_ref().is_some_and(|n| n.id == id) {
            if mode == OpenMode::History {
                self.tabs[self.active_tab].note = Some(id);
            }
            return;
        }
        self.flush_save();
        self.open_mode = mode;
        self.db.send(Cmd::Load { id, purpose: LoadPurpose::Open });
    }

    fn activate_tab(&mut self, i: usize) {
        self.active_tab = i.min(self.tabs.len() - 1);
        if self.tabs[self.active_tab].dashboard {
            self.flush_save();
            self.open = None;
            self.need.board = true;
            return;
        }
        match self.tabs[self.active_tab].note {
            Some(id) => self.open_in(id, OpenMode::History),
            None => {
                self.flush_save();
                self.open = None;
            }
        }
    }

    fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let was_active = i == self.active_tab;
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::default());
        }
        if i < self.active_tab || self.active_tab >= self.tabs.len() {
            self.active_tab = self.active_tab.saturating_sub(1);
        }
        if was_active {
            self.activate_tab(self.active_tab);
        }
    }

    /// A note was deleted: drop it from tabs and histories.
    fn forget_note(&mut self, id: Uuid) {
        for t in &mut self.tabs {
            t.back.retain(|x| *x != id);
            t.fwd.retain(|x| *x != id);
            if t.note == Some(id) {
                t.note = None;
            }
        }
    }

    fn navigate(&mut self, forward: bool) {
        let t = &mut self.tabs[self.active_tab];
        let target = if forward { t.fwd.pop() } else { t.back.pop() };
        if let Some(target) = target {
            if let Some(cur) = t.note {
                if forward { t.back.push(cur) } else { t.fwd.push(cur) }
            }
            self.open_in(target, OpenMode::History);
        }
    }

    fn tab_title(&self, t: &Tab) -> String {
        if t.dashboard {
            return "Tableau de bord".into();
        }
        match t.note {
            None => "Nouvel onglet".into(),
            Some(id) => self
                .notes
                .iter()
                .find(|n| n.id == id)
                .map(|n| basename(&n.path).to_string())
                .or_else(|| self.open.as_ref().filter(|o| o.id == id).map(|o| basename(&o.path).to_string()))
                .unwrap_or_else(|| "…".into()),
        }
    }

    fn flush_save(&mut self) {
        if let Some(n) = &mut self.open
            && n.saving.is_none()
            && n.conflict.is_none()
            && n.content != n.saved
        {
            n.saving = Some(n.content.clone());
            n.last_edit = None;
            self.db.send(Cmd::Save { id: n.id, content: n.content.clone(), base_version: n.version });
        }
    }

    fn tick_autosave(&mut self, ctx: &egui::Context, now: f64) {
        let Some(n) = &self.open else { return };
        if n.saving.is_some() || n.conflict.is_some() || n.content == n.saved {
            return;
        }
        if let Some(t) = n.last_edit {
            if now - t >= AUTOSAVE_DELAY {
                self.flush_save();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(AUTOSAVE_DELAY - (now - t) + 0.01));
            }
        }
    }

    fn rebuild_tree(&mut self) {
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
        fn sort(f: &mut TreeFolder) {
            f.notes.sort_by_key(|n| n.0.to_lowercase());
            f.folders.values_mut().for_each(sort);
        }
        sort(&mut root);
        self.tree = root;
    }

    fn note_ctx(&self) -> Option<(String, String, String)> {
        self.open.as_ref().map(|n| (n.id.to_string(), n.path.clone(), n.content.clone()))
    }

    // ------------------------------------------------------------------ plugin actions
    fn apply_plugin_actions(&mut self, ctx: &egui::Context, now: f64) {
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

    fn insert_at_cursor(&mut self, ctx: &egui::Context, text: &str, now: f64) {
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

    // ------------------------------------------------------------------ actions
    fn process_actions(&mut self, ctx: &egui::Context, now: f64) {
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
            }
        }
    }

    fn run_core(&mut self, c: CoreCmd, ctx: &egui::Context, now: f64) {
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
            CoreCmd::Graph => {
                self.graph.open = !self.graph.open;
                if self.graph.open {
                    self.need.graph = true;
                }
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
                self.flush_save();
                self.tabs.push(Tab::default());
                self.active_tab = self.tabs.len() - 1;
                self.open = None;
            }
            CoreCmd::CloseTab => self.close_tab(self.active_tab),
            CoreCmd::Dashboard => self.open_dashboard(None),
            CoreCmd::DailyNote => match self.plugin_cmds.iter().find(|c| c.1 == "today").cloned() {
                Some((i, id, _)) => {
                    self.actions.push(UiAction::Plugin(i, id));
                    self.process_actions(ctx, now);
                }
                None => self.notice("Activez le plugin « Notes quotidiennes » dans les paramètres", false, now),
            },
            CoreCmd::Templates => {
                self.dialog = Some(Dialog::Palette { query: "Modèles".into(), sel: 0 });
                self.focus_dialog = true;
            }
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
                    graph: self.graph.open,
                    board: true,
                };
                if let Some(n) = &self.open {
                    self.db.send(Cmd::Load { id: n.id, purpose: LoadPurpose::Refresh });
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
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

    fn apply_theme(&mut self, ctx: &egui::Context) {
        let key = (self.cfg.dark_mode, self.cfg.font_size, theme::generation());
        if self.theme_applied == Some(key) {
            return;
        }
        self.theme_applied = Some(key);
        theme::apply(ctx, self.cfg.dark_mode, 14.0);
    }

    fn pal(&self) -> theme::Palette {
        theme::palette(self.cfg.dark_mode)
    }

    // ------------------------------------------------------------------ UI parts
    fn ribbon(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(5.0);
            if theme::icon_button(ui, icon::PANEL_LEFT, "Afficher / masquer le panneau latéral", false).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::ToggleLeft));
            }
            ui.add_space(10.0);
            let items: [(&str, &str, CoreCmd); 6] = [
                (icon::DASHBOARD, "Tableau de bord : projets et tickets (Ctrl+Shift+D)", CoreCmd::Dashboard),
                (icon::SWITCHER, "Ouvrir le sélecteur rapide", CoreCmd::Switcher),
                (icon::GRAPH, "Ouvrir la vue graphique", CoreCmd::Graph),
                (icon::DAILY, "Ouvrir la note du jour", CoreCmd::DailyNote),
                (icon::TEMPLATES, "Insérer un modèle", CoreCmd::Templates),
                (icon::PALETTE, "Ouvrir la palette de commandes", CoreCmd::Palette),
            ];
            for (ic, tip, c) in items {
                if theme::icon_button(ui, ic, tip, false).clicked() {
                    self.actions.push(UiAction::Core(c));
                }
                ui.add_space(2.0);
            }
        });
    }

    /// Name shown in the vault footer: the database name.
    fn vault_name(&self) -> String {
        let url = &self.cfg.database_url;
        url.rsplit('/')
            .next()
            .and_then(|s| s.split('?').next())
            .filter(|s| !s.is_empty() && !s.contains('@'))
            .unwrap_or("Mnemo")
            .to_string()
    }

    fn left_sidebar(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.pal();
        // header: view tabs
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 40.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(6.0);
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
        let footer_h = 42.0;
        let body_h = (ui.available_height() - footer_h).max(50.0);
        ui.allocate_ui(egui::vec2(ui.available_width(), body_h), |ui| {
            ui.set_min_height(body_h);
            match self.left_tab {
                LeftTab::Files => {
                    ui.horizontal(|ui| {
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
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        let open_id = self.open.as_ref().map(|n| n.id);
                        let tree = std::mem::take(&mut self.tree);
                        let mut actions = vec![];
                        tree_ui(ui, &tree, open_id, &mut actions, 0, &self.expanded, self.sort);
                        self.tree = tree;
                        self.actions.extend(actions);
                        let rest =
                            ui.allocate_response(ui.available_size().max(egui::vec2(10.0, 40.0)), egui::Sense::click());
                        rest.context_menu(|ui| {
                            if ui.button("Nouvelle note").clicked() {
                                self.actions.push(UiAction::Core(CoreCmd::NewNote));
                                ui.close();
                            }
                            if ui.button("Nouveau dossier").clicked() {
                                self.actions.push(UiAction::Core(CoreCmd::NewFolder));
                                ui.close();
                            }
                        });
                    });
                }
                LeftTab::Search => {
                    ui.add_space(2.0);
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 0)).show(ui, |ui| {
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
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 0)).show(ui, |ui| {
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
                        });
                    });
                }
                LeftTab::Tags => {
                    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            if self.tags.is_empty() {
                                ui.label(RichText::new("Aucun tag. Écrivez #montag dans une note.").color(p.faint));
                            }
                            for (tag, count) in &self.tags {
                                if theme::suggestion_row(ui, &format!("#{tag}"), &count.to_string(), false).clicked() {
                                    self.actions.push(UiAction::OpenTag(tag.clone()));
                                }
                            }
                        });
                    });
                }
            }
        });
        // vault footer
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
        ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, p.border));
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), footer_h - 2.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(6.0);
                let name = self.vault_name();
                let r = ui
                    .add(egui::Button::new(RichText::new(format!("{name}  ")).color(p.text).strong()).frame(false))
                    .on_hover_text(format!("Base : {}", redact_url(&self.cfg.database_url)));
                ui.label(theme::icon_text(icon::VAULT_SWITCH, 13.0).color(p.faint));
                if r.clicked() {
                    self.settings_tab = SettingsTab::Connection;
                    self.show_settings = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
                    if theme::icon_button_sized(ui, icon::SETTINGS, "Paramètres", self.show_settings, true, 16.0)
                        .clicked()
                    {
                        self.actions.push(UiAction::Core(CoreCmd::Settings));
                    }
                    if theme::icon_button_sized(ui, icon::HELP, "Aide et raccourcis", false, true, 16.0).clicked() {
                        self.settings_tab = SettingsTab::About;
                        self.show_settings = true;
                    }
                });
            },
        );
    }

    fn right_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
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
            egui::vec2(ui.available_width(), 40.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.add_space(6.0);
                for (i, (ic, name)) in tabs.iter().enumerate() {
                    if theme::icon_button_sized(ui, ic, name, self.right_tab == i, true, 16.0).clicked() {
                        self.right_tab = i;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(4.0);
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
            egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if self.open.is_none() {
                    ui.label(RichText::new("Aucun fichier actif.").color(p.faint));
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

    /// Small floating status bar in the bottom-right corner.
    fn status_bar(&mut self, ctx: &egui::Context) {
        let p = self.pal();
        egui::Area::new(egui::Id::unique("status-bar"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(0.0, 0.0))
            .order(egui::Order::Middle)
            .interactable(true)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.bg2)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(egui::CornerRadius { nw: 8, ne: 0, sw: 0, se: 0 })
                    .inner_margin(egui::Margin { left: 10, right: 10, top: 3, bottom: 4 })
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 14.0;
                            let small = |t: String| RichText::new(t).size(12.0).color(p.muted);
                            if let Some(n) = &self.open {
                                if n.conflict.is_some() {
                                    ui.label(
                                        RichText::new("Conflit à résoudre")
                                            .size(12.0)
                                            .color(Color32::from_rgb(0xe9, 0x97, 0x3f)),
                                    );
                                }
                                ui.label(small(format!(
                                    "{} rétrolien{}",
                                    self.backlinks.len(),
                                    if self.backlinks.len() > 1 { "s" } else { "" }
                                )));
                                for s in &self.plugin_status {
                                    ui.label(small(s.clone()));
                                }
                                let now = ui.input(|i| i.time);
                                if now - n.remote_flash < 3.0 {
                                    ui.label(RichText::new("mis à jour à distance").size(12.0).color(p.accent));
                                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                                }
                            }
                            let (ic, color, tip) = match (&self.conn, self.live, self.open.as_ref().map(|n| n.dirty()))
                            {
                                (ConnState::Connected, true, Some(true)) => {
                                    (icon::SYNC, p.faint, "Enregistrement…".to_string())
                                }
                                (ConnState::Connected, true, _) => (
                                    icon::SYNC,
                                    Color32::from_rgb(0x44, 0xcf, 0x6e),
                                    "Synchronisé en direct".to_string(),
                                ),
                                (ConnState::Connected, false, _) => (
                                    icon::SYNC,
                                    Color32::from_rgb(0xe9, 0x97, 0x3f),
                                    "Connecté, flux live en reconnexion…".to_string(),
                                ),
                                _ => (icon::OFFLINE, Color32::from_rgb(0xfb, 0x46, 0x4c), "Hors ligne".to_string()),
                            };
                            ui.label(theme::icon_text(ic, 13.0).color(color))
                                .on_hover_text(format!("{tip}\n{}", redact_url(&self.cfg.database_url)));
                        });
                    });
            });
    }

    fn connection_screen(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.15);
            ui.heading(RichText::new("Mnemo").size(34.0).strong());
            ui.label("Vos notes Markdown, stockées dans PostgreSQL et synchronisées en direct.");
            ui.add_space(24.0);
            ui.allocate_ui(egui::vec2(560.0, 300.0), |ui| {
                ui.label("URL de connexion PostgreSQL");
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut self.url_input)
                            .password(!self.show_password)
                            .desired_width(470.0)
                            .hint_text("postgres://user:motdepasse@hôte:5432/base"),
                    );
                    ui.checkbox(&mut self.show_password, "👁");
                    if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        self.connect();
                    }
                });
                ui.add_space(6.0);
                let connecting = self.conn == ConnState::Connecting;
                if ui.add_enabled(!connecting, egui::Button::new(if connecting { "Connexion…" } else { "Se connecter" })).clicked() {
                    self.connect();
                }
                if let ConnState::Disconnected(Some(e)) = &self.conn {
                    ui.add_space(8.0);
                    ui.colored_label(Color32::from_rgb(230, 90, 90), format!("Échec : {e}"));
                }
                ui.add_space(16.0);
                ui.weak("Pas encore de base ? Lancez-en une avec Docker :");
                ui.code("docker run -d --name mnemo-db -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=vault -p 5432:5432 postgres:16");
                ui.weak("Pour un serveur distant (Neon, Supabase, RDS…), ajoutez ?sslmode=require à l'URL.");
                ui.weak("Toutes les instances connectées à la même base se synchronisent en temps réel.");
            });
        });
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, p.bg2);
        let mut child =
            ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Max)));
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 1.0;
        ui.add_space(6.0);
        let right_reserved = 44.0;
        let titles: Vec<String> = self.tabs.iter().map(|t| self.tab_title(t)).collect();
        for (i, title) in titles.iter().enumerate() {
            if ui.available_width() < right_reserved + 60.0 {
                break;
            }
            let active = i == self.active_tab;
            let dirty = active && self.open.as_ref().is_some_and(|n| n.dirty());
            let (r, close) = theme::tab(ui, title, active, dirty);
            if close {
                self.actions.push(UiAction::CloseTab(i));
            } else if r.clicked() {
                self.actions.push(UiAction::SwitchTab(i));
            }
            r.context_menu(|ui| {
                if ui.button("Fermer").clicked() {
                    self.actions.push(UiAction::CloseTab(i));
                    ui.close();
                }
                if ui.button("Fermer les autres onglets").clicked() {
                    self.actions.push(UiAction::CloseOtherTabs(i));
                    ui.close();
                }
            });
        }
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(4.0);
            if theme::icon_button_sized(ui, icon::PLUS, "Nouvel onglet", false, true, 16.0).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::NewTab));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(6.0);
                if !self.right_visible(ui.ctx())
                    && theme::icon_button_sized(
                        ui,
                        icon::PANEL_RIGHT,
                        "Afficher le panneau de droite",
                        false,
                        true,
                        16.0,
                    )
                    .clicked()
                {
                    self.actions.push(UiAction::Core(CoreCmd::ToggleRight));
                }
            });
        });
    }

    fn view_header(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let Some(n) = &self.open else { return };
        let (id, path) = (n.id, n.path.clone());
        let tab = &self.tabs[self.active_tab];
        let (can_back, can_fwd) = (!tab.back.is_empty(), !tab.fwd.is_empty());
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), egui::Sense::hover());
        // centered breadcrumb: folders faint, note title normal
        let mut job = LayoutJob::default();
        let fmt = |c: Color32| TextFormat { font_id: FontId::proportional(14.0), color: c, ..Default::default() };
        if let Some(parent) = parent(&path) {
            for seg in parent.split('/') {
                job.append(seg, 0.0, fmt(p.faint));
                job.append("  /  ", 0.0, fmt(p.faint));
            }
        }
        job.append(basename(&path), 0.0, fmt(p.text));
        let galley = ui.painter().layout_job(job);
        let crumb = egui::Rect::from_center_size(rect.center(), galley.size());
        ui.painter().with_clip_rect(rect.shrink2(egui::vec2(90.0, 0.0))).galley(crumb.min, galley, p.text);
        let crumb_resp = ui.interact(crumb, egui::Id::unique(("crumb", id)), egui::Sense::click());
        if crumb_resp.double_clicked() {
            self.actions.push(UiAction::RenameNote(id, path.clone()));
        }
        crumb_resp.on_hover_text(format!("{path}\nDouble-clic pour déplacer"));

        let mut child = ui
            .new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 2.0;
        ui.add_space(8.0);
        if theme::icon_button_sized(ui, icon::BACK, "Revenir en arrière", false, can_back, 16.0).clicked() {
            self.actions.push(UiAction::Back);
        }
        if theme::icon_button_sized(ui, icon::FORWARD, "Avancer", false, can_fwd, 16.0).clicked() {
            self.actions.push(UiAction::Forward);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            let more = theme::icon_button_sized(ui, icon::MORE, "Plus d'options", false, true, 16.0);
            egui::Popup::menu(&more).show(|ui| {
                ui.set_min_width(200.0);
                if ui.button("Renommer / déplacer…").clicked() {
                    self.actions.push(UiAction::RenameNote(id, path.clone()));
                    ui.close();
                }
                if ui.button("Copier le lien [[…]]").clicked() {
                    ui.ctx().copy_text(format!("[[{}]]", basename(&path)));
                    ui.close();
                }
                if ui.button("Mode divisé (édition + lecture)").clicked() {
                    self.actions.push(UiAction::Core(CoreCmd::SplitMode));
                    ui.close();
                }
                ui.separator();
                if ui.button(RichText::new("Supprimer le fichier").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked()
                {
                    self.actions.push(UiAction::DeleteNote(id, path.clone()));
                    ui.close();
                }
            });
            let reading = self.cfg.view_mode == ViewMode::Preview;
            let (ic, tip) = if reading {
                (icon::EDITING, "Basculer en mode édition (Ctrl+E)")
            } else {
                (icon::READING, "Basculer en mode lecture (Ctrl+E)")
            };
            if theme::icon_button_sized(ui, ic, tip, false, true, 16.0).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::ToggleMode));
            }
            if self.cfg.view_mode == ViewMode::Split
                && theme::icon_button_sized(ui, icon::SPLIT, "Quitter le mode divisé", true, true, 16.0).clicked()
            {
                self.cfg.view_mode = ViewMode::Edit;
                self.cfg.save();
            }
        });
    }

    fn empty_tab(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.32).max(20.0));
            ui.label(RichText::new("Aucun fichier n'est ouvert").size(22.0).color(p.muted));
            ui.add_space(18.0);
            let link = |ui: &mut egui::Ui, t: &str| {
                ui.add(egui::Label::new(RichText::new(t).size(15.0).color(p.link)).sense(egui::Sense::click()))
            };
            if link(ui, "Créer une nouvelle note (Ctrl+N)").clicked() {
                self.actions.push(UiAction::Core(CoreCmd::NewNote));
            }
            ui.add_space(6.0);
            if link(ui, "Aller au fichier (Ctrl+O)").clicked() {
                self.actions.push(UiAction::Core(CoreCmd::Switcher));
            }
            ui.add_space(6.0);
            if self.tabs.len() > 1 && link(ui, "Fermer").clicked() {
                self.actions.push(UiAction::Core(CoreCmd::CloseTab));
            }
        });
    }

    fn metrics(&self) -> theme::Metrics {
        theme::metrics(self.cfg.dark_mode)
    }

    /// Editor / reading font size: CSS `--font-text-size` (or appearance.json) wins over the setting.
    fn text_size(&self) -> f32 {
        self.metrics().font_size.unwrap_or(self.cfg.font_size)
    }

    fn readable_width(&self) -> f32 {
        self.metrics().line_width.unwrap_or(700.0 * self.text_size() / 16.0)
    }

    fn dialogs(&mut self, ctx: &egui::Context, now: f64) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut close = false;
        let focus = std::mem::take(&mut self.focus_dialog);
        let prompt = matches!(dialog, Dialog::Switcher { .. } | Dialog::Palette { .. });
        let p = self.pal();
        let mid = egui::Id::unique("mnemo-dialog");
        let mut modal = egui::Modal::new(mid)
            .backdrop_color(Color32::from_black_alpha(if self.cfg.dark_mode { 110 } else { 50 }))
            .frame(
                egui::Frame::new()
                    .fill(p.bg2)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(10.0)
                    .inner_margin(if prompt { egui::Margin::ZERO } else { egui::Margin::same(20) })
                    .shadow(ctx.global_style().visuals.window_shadow),
            );
        if prompt {
            modal = modal.area(egui::Modal::default_area(mid).anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0)));
        }
        let modal = modal.show(ctx, |ui| {
            ui.set_width(if prompt { 640.0 } else { 460.0 });
            let enter = ui.input(|i| i.key_pressed(Key::Enter));
            let prompt_field = |ui: &mut egui::Ui, s: &mut String, hint: &str| {
                let r = egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 13))
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(s)
                                .hint_text(RichText::new(hint).color(p.faint))
                                .font(FontId::proportional(16.0))
                                .text_color(p.text)
                                .frame(egui::Frame::NONE)
                                .desired_width(f32::INFINITY),
                        )
                    })
                    .inner;
                if focus {
                    r.request_focus();
                }
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
                ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, p.border));
                ui.add_space(6.0);
                r
            };
            let text_field = |ui: &mut egui::Ui, s: &mut String, hint: &str| {
                let r = ui.add(egui::TextEdit::singleline(s).hint_text(hint).desired_width(f32::INFINITY));
                if focus {
                    r.request_focus();
                }
                r
            };
            match &mut dialog {
                Dialog::NewNote { input, error } => {
                    ui.heading("Nouvelle note");
                    ui.weak("Utilisez « / » pour ranger dans un dossier, ex : Projets/Idée");
                    text_field(ui, input, "Dossier/Nom de la note");
                    if let Some(e) = error {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Créer").clicked() || enter {
                            match normalize_path(input) {
                                Ok(path) => {
                                    if self.notes.iter().any(|n| n.path.eq_ignore_ascii_case(&path)) {
                                        *error = Some("Une note porte déjà ce nom".into());
                                    } else {
                                        let content = String::new();
                                        self.flush_save();
                                        self.db.send(Cmd::Create { path, content, open: true, if_missing: false });
                                        close = true;
                                    }
                                }
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::NewFolder { input, error } => {
                    ui.heading("Nouveau dossier");
                    text_field(ui, input, "Dossier/Sous-dossier");
                    if let Some(e) = error {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Créer").clicked() || enter {
                            match normalize_path(input) {
                                Ok(p) => {
                                    self.db.send(Cmd::CreateFolder(p));
                                    close = true;
                                }
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::RenameNote { id, input, error } => {
                    ui.heading("Renommer / déplacer");
                    ui.weak("Les liens [[…]] des autres notes seront mis à jour.");
                    text_field(ui, input, "Dossier/Nouveau nom");
                    if let Some(e) = error {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Renommer").clicked() || enter {
                            match normalize_path(input) {
                                Ok(p) => {
                                    self.flush_save();
                                    self.db.send(Cmd::Rename { id: *id, new_path: p });
                                    close = true;
                                }
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::RenameFolder { from, input, error } => {
                    ui.heading(format!("Renommer le dossier « {from} »"));
                    text_field(ui, input, "Nouveau chemin");
                    if let Some(e) = error {
                        ui.colored_label(Color32::from_rgb(230, 90, 90), e.as_str());
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Renommer").clicked() || enter {
                            match normalize_path(input) {
                                Ok(p) => {
                                    self.flush_save();
                                    self.db.send(Cmd::RenameFolder { from: from.clone(), to: p });
                                    close = true;
                                }
                                Err(e) => *error = Some(e),
                            }
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::DeleteNote { id, path } => {
                    ui.heading("Supprimer la note ?");
                    ui.label(format!("« {path} » sera supprimée définitivement de la base, pour tous les appareils."));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(230, 90, 90))).clicked() {
                            if self.open.as_ref().is_some_and(|n| n.id == *id) {
                                self.open = None;
                            }
                            self.db.send(Cmd::Delete(*id));
                            close = true;
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::DeleteFolder { path } => {
                    let count = self
                        .notes
                        .iter()
                        .filter(|n| n.path.to_lowercase().starts_with(&(path.to_lowercase() + "/")))
                        .count();
                    ui.heading("Supprimer le dossier ?");
                    ui.label(format!("« {path} » et ses {count} note(s) seront supprimés définitivement."));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(230, 90, 90))).clicked() {
                            if self
                                .open
                                .as_ref()
                                .is_some_and(|n| n.path.to_lowercase().starts_with(&(path.to_lowercase() + "/")))
                            {
                                self.open = None;
                            }
                            self.db.send(Cmd::DeleteFolder(path.clone()));
                            close = true;
                        }
                        if ui.button("Annuler").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::Switcher { query, sel } => {
                    let q = fold(query);
                    let words: Vec<&str> = q.split_whitespace().collect();
                    let mut items: Vec<&NoteMeta> =
                        self.notes.iter().filter(|n| words.iter().all(|w| fold(&n.path).contains(w))).collect();
                    items.sort_by_key(|n| (!fold(basename(&n.path)).starts_with(&q), std::cmp::Reverse(n.updated_at)));
                    items.truncate(12);
                    let create =
                        !query.trim().is_empty() && !items.iter().any(|n| n.path.eq_ignore_ascii_case(query.trim()));
                    nav_keys(ui, sel, items.len() + usize::from(create));
                    let r = prompt_field(ui, query, "Trouver ou créer une note…");
                    if r.changed() {
                        *sel = 0;
                    }
                    let mut chosen: Option<Option<Uuid>> = None;
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, n) in items.iter().enumerate() {
                            let r =
                                theme::suggestion_row(ui, basename(&n.path), parent(&n.path).unwrap_or(""), *sel == i);
                            if r.clicked() || (enter && *sel == i) {
                                chosen = Some(Some(n.id));
                            }
                        }
                        if create {
                            let i = items.len();
                            let r = theme::suggestion_row(ui, query.trim(), "Entrée pour créer", *sel == i);
                            if r.clicked() || (enter && *sel == i) {
                                chosen = Some(None);
                            }
                        }
                        if items.is_empty() && !create {
                            ui.add_space(6.0);
                            ui.label(RichText::new("  Aucun résultat.").color(p.faint));
                        }
                    });
                    prompt_footer(
                        ui,
                        &p,
                        &[
                            ("↑↓", "pour naviguer"),
                            ("↵", "pour ouvrir"),
                            ("ctrl ↵", "dans un nouvel onglet"),
                            ("échap", "pour annuler"),
                        ],
                    );
                    let new_tab = ui.input(|i| i.modifiers.command);
                    match chosen {
                        Some(Some(id)) => {
                            self.actions.push(if new_tab { UiAction::OpenNewTab(id) } else { UiAction::OpenId(id) });
                            close = true;
                        }
                        Some(None) => {
                            self.actions.push(UiAction::OpenLink(query.trim().to_string()));
                            close = true;
                        }
                        None => {}
                    }
                }
                Dialog::Palette { query, sel } => {
                    let q = fold(query);
                    let words: Vec<&str> = q.split_whitespace().collect();
                    let mut items: Vec<(String, String, UiAction)> = CORE_COMMANDS
                        .iter()
                        .map(|(c, name, key)| (name.to_string(), key.to_string(), UiAction::Core(*c)))
                        .chain(
                            self.plugin_cmds
                                .iter()
                                .map(|(i, id, name)| (name.clone(), String::new(), UiAction::Plugin(*i, id.clone()))),
                        )
                        .filter(|(name, _, _)| words.iter().all(|w| fold(name).contains(w)))
                        .collect();
                    items.truncate(14);
                    nav_keys(ui, sel, items.len());
                    let r = prompt_field(ui, query, "Tapez une commande…");
                    if r.changed() {
                        *sel = 0;
                    }
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, (name, key, action)) in items.iter().enumerate() {
                            if theme::suggestion_row(ui, name, key, *sel == i).clicked() || (enter && *sel == i) {
                                self.actions.push(action.clone());
                                close = true;
                            }
                        }
                        if items.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("  Aucune commande trouvée.").color(p.faint));
                        }
                    });
                    prompt_footer(
                        ui,
                        &p,
                        &[("↑↓", "pour naviguer"), ("↵", "pour utiliser"), ("échap", "pour annuler")],
                    );
                }
            }
        });
        if modal.should_close() {
            close = true;
        }
        if !close {
            self.dialog = Some(dialog);
        }
        let _ = now;
    }

    /// Side panels auto-hide on narrow windows unless explicitly toggled
    fn left_visible(&self, ctx: &egui::Context) -> bool {
        self.cfg.show_left && ctx.content_rect().width() >= 560.0
    }
    fn right_visible(&self, ctx: &egui::Context) -> bool {
        self.cfg.show_right && ctx.content_rect().width() >= 1000.0
    }

    fn update_plugin_outputs(&mut self, ctx: &egui::Context, now: f64) {
        let key = (self.content_gen, self.plugin_gen);
        if (self.plugin_computed.0, self.plugin_computed.1) == key {
            return;
        }
        if now - self.plugin_computed.2 < 0.15 {
            ctx.request_repaint_after(std::time::Duration::from_millis(160));
            return;
        }
        self.plugin_computed = (key.0, key.1, now);
        let nc = self.note_ctx();
        let note = nc.as_ref().map(|(id, path, content)| NoteCtx { id: id.clone(), path, content });
        self.plugin_status = self.plugins.status(note.as_ref());
        self.plugin_panels = self.plugins.panels(note.as_ref());
    }
}

fn prompt_footer(ui: &mut egui::Ui, p: &theme::Palette, items: &[(&str, &str)]) {
    ui.add_space(6.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().hline(rect.x_range(), rect.center().y, egui::Stroke::new(1.0, p.border));
    egui::Frame::new().inner_margin(egui::Margin::symmetric(16, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            for (k, label) in items {
                ui.label(RichText::new(*k).size(12.0).color(p.muted).strong());
                ui.label(RichText::new(*label).size(12.0).color(p.faint));
                ui.add_space(10.0);
            }
        });
    });
}

fn nav_keys(ui: &mut egui::Ui, sel: &mut usize, len: usize) {
    if len == 0 {
        return;
    }
    ui.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            *sel = (*sel + 1) % len;
        }
        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            *sel = (*sel + len - 1) % len;
        }
    });
    *sel = (*sel).min(len - 1);
}

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

/// Lays out a column of at most `max_w`, horizontally centered (readable line length).
fn centered<R>(ui: &mut egui::Ui, max_w: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let avail = ui.available_width();
    let w = (avail - 48.0).min(max_w).max(120.0);
    let pad = ((avail - w) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(pad);
        ui.vertical(|ui| {
            ui.set_width(w);
            add(ui)
        })
        .inner
    })
    .inner
}

fn tree_ui(
    ui: &mut egui::Ui,
    folder: &TreeFolder,
    open: Option<Uuid>,
    actions: &mut Vec<UiAction>,
    depth: usize,
    expanded: &std::collections::HashSet<String>,
    sort: SortOrder,
) {
    let mut folders: Vec<&TreeFolder> = folder.folders.values().collect();
    if sort == SortOrder::NameDesc {
        folders.reverse();
    }
    for sub in folders {
        let is_open = expanded.contains(&sub.path.to_lowercase());
        let r = theme::tree_row(ui, &sub.name, depth, false, Some(is_open));
        if r.clicked() {
            actions.push(UiAction::ToggleFolder(sub.path.clone()));
        }
        r.context_menu(|ui| {
            if ui.button("Nouvelle note").clicked() {
                actions.push(UiAction::NewNoteIn(sub.path.clone()));
                ui.close();
            }
            if ui.button("Nouveau dossier").clicked() {
                actions.push(UiAction::NewFolderIn(sub.path.clone()));
                ui.close();
            }
            if ui.button("Renommer / déplacer…").clicked() {
                actions.push(UiAction::RenameFolder(sub.path.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                actions.push(UiAction::DeleteFolder(sub.path.clone()));
                ui.close();
            }
        });
        if is_open {
            tree_ui(ui, sub, open, actions, depth + 1, expanded, sort);
        }
    }
    let mut notes: Vec<&(String, Uuid, chrono::DateTime<chrono::Utc>)> = folder.notes.iter().collect();
    match sort {
        SortOrder::NameAsc => {}
        SortOrder::NameDesc => notes.reverse(),
        SortOrder::Modified => notes.sort_by_key(|n| std::cmp::Reverse(n.2)),
    }
    for (name, id, _) in notes {
        let r = theme::tree_row(ui, name, depth, open == Some(*id), None);
        if r.clicked() {
            if ui.input(|i| i.modifiers.command) {
                actions.push(UiAction::OpenNewTab(*id));
            } else {
                actions.push(UiAction::OpenId(*id));
            }
        }
        if r.middle_clicked() {
            actions.push(UiAction::OpenNewTab(*id));
        }
        let path = if folder.path.is_empty() { name.clone() } else { format!("{}/{}", folder.path, name) };
        r.context_menu(|ui| {
            if ui.button("Ouvrir dans un nouvel onglet").clicked() {
                actions.push(UiAction::OpenNewTab(*id));
                ui.close();
            }
            if ui.button("Renommer / déplacer…").clicked() {
                actions.push(UiAction::RenameNote(*id, path.clone()));
                ui.close();
            }
            if ui.button("Copier le lien [[…]]").clicked() {
                ui.ctx().copy_text(format!("[[{name}]]"));
                ui.close();
            }
            ui.separator();
            if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                actions.push(UiAction::DeleteNote(*id, path.clone()));
                ui.close();
            }
        });
    }
}

impl eframe::App for MnemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.watch_css(&ctx, now);
        self.apply_theme(&ctx);
        self.handle_messages(now);
        self.apply_plugin_actions(&ctx, now);
        self.tick_autosave(&ctx, now);
        if let Some(t) = self.search_due {
            if now >= t {
                self.search_due = None;
                if !self.search_q.trim().is_empty() && self.conn == ConnState::Connected {
                    self.db.send(Cmd::Search(self.search_q.clone()));
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(t - now));
            }
        }
        self.flush_needs();

        // window title
        let title = match &self.open {
            Some(n) => format!("{} — Mnemo", basename(&n.path)),
            None => "Mnemo".into(),
        };
        if title != self.title_shown {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title_shown = title;
        }

        if self.conn != ConnState::Connected {
            egui::CentralPanel::default().show(ui, |ui| self.connection_screen(ui));
            self.notices_ui(&ctx, now);
            return;
        }

        self.shortcuts(&ctx);
        self.update_plugin_outputs(&ctx, now);

        let p = self.pal();
        egui::Panel::left("ribbon")
            .exact_size(44.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(p.bg2))
            .show(ui, |ui| self.ribbon(ui));
        if self.left_visible(&ctx) {
            egui::Panel::left("sidebar")
                .default_size(260.0)
                .size_range(180.0..=520.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(p.bg2))
                .show(ui, |ui| self.left_sidebar(ui, now));
        }
        if self.right_visible(&ctx) {
            egui::Panel::right("right")
                .default_size(280.0)
                .size_range(180.0..=520.0)
                .resizable(true)
                .frame(egui::Frame::new().fill(p.bg2))
                .show(ui, |ui| self.right_sidebar(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ui, |ui| self.editor_area(ui, now));
        self.status_bar(&ctx);

        if self.graph.open {
            let mut open = true;
            let current = self.open.as_ref().map(|n| n.id.to_string());
            let mut clicked = None;
            egui::Window::new("Graphe").open(&mut open).default_size([720.0, 520.0]).show(&ctx, |ui| {
                if !self.graph.loaded {
                    ui.spinner();
                }
                clicked = self.graph.ui(ui, current.as_deref());
            });
            self.graph.open = open;
            if let Some(c) = clicked {
                self.actions.push(UiAction::OpenLink(c));
            }
        }
        if self.show_settings {
            self.settings_window(&ctx, now);
        }
        self.conflict_window(&ctx, now);
        self.dialogs(&ctx, now);
        self.notices_ui(&ctx, now);

        // Intercept clicks on [[wikilinks]] / #tags rendered in the preview
        let mut clicked: Vec<String> = vec![];
        ctx.output_mut(|o| {
            o.commands.retain(|c| match c {
                egui::OutputCommand::OpenUrl(u) if u.url.starts_with("mnemo:") => {
                    clicked.push(u.url.clone());
                    false
                }
                _ => true,
            })
        });
        for url in clicked {
            if let Some(t) = url.strip_prefix(markdown::NOTE_SCHEME) {
                self.actions.push(UiAction::OpenLink(markdown::decode(t)));
            } else if let Some(t) = url.strip_prefix(markdown::TAG_SCHEME) {
                self.actions.push(UiAction::OpenTag(markdown::decode(t)));
            }
        }
        self.process_actions(&ctx, now);
        self.apply_plugin_actions(&ctx, now);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Unresolved conflict: never lose the local text, keep it as a separate note
        if let Some(n) = self.open.as_ref().filter(|n| n.conflict.is_some()) {
            let path = format!("{} (conflit {})", n.path, chrono::Local::now().format("%Y-%m-%d %H-%M"));
            self.db.send(Cmd::Create { path, content: n.content.clone(), open: false, if_missing: false });
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                match self.rx.recv_timeout(left) {
                    Ok(Msg::Created { .. }) | Ok(Msg::Error(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        }
        // Last chance to persist unsaved text
        self.flush_save();
        if self.open.as_ref().is_some_and(|n| n.saving.is_some()) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                match self.rx.recv_timeout(left) {
                    Ok(Msg::Saved(_)) | Ok(Msg::Conflict(_)) | Ok(Msg::Error(_)) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        self.cfg.save();
    }
}

impl MnemoApp {
    fn notices_ui(&mut self, ctx: &egui::Context, now: f64) {
        self.notices.retain(|n| n.1 > now);
        if self.notices.is_empty() {
            return;
        }
        let next = self.notices.iter().map(|n| n.1).fold(f64::MAX, f64::min);
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((next - now).max(0.05)));
        egui::Area::new(egui::Id::unique("notices"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                for (msg, _, err) in &self.notices {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_max_width(360.0);
                        let t = RichText::new(msg);
                        ui.label(if *err { t.color(Color32::from_rgb(240, 110, 110)) } else { t });
                    });
                    ui.add_space(6.0);
                }
            });
    }
}

const WELCOME: &str = include_str!("../assets/texts/welcome.md");
