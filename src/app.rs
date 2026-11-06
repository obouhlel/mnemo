mod dashboard;

use crate::board::Board;
use crate::config::{Config, ViewMode, redact_url};
use crate::db::{Cmd, DbHandle, LoadPurpose, Msg};
use crate::graph::GraphView;
use crate::markdown;
use crate::merge::{self, Block, Choice, Origin};
use crate::model::*;
use crate::plugins::{Action, NoteCtx, PluginHost, toml_to_json};
use crate::theme::{self, icon};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Color32, FontId, Key, KeyboardShortcut, Modifiers, RichText, TextFormat};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::Receiver;
use uuid::Uuid;

const AUTOSAVE_DELAY: f64 = 0.6;

#[derive(PartialEq)]
enum ConnState {
    Disconnected(Option<String>),
    Connecting,
    Connected,
}

#[derive(PartialEq, Clone, Copy)]
enum LeftTab {
    Files,
    Search,
    Tags,
}

#[derive(PartialEq, Clone, Copy)]
enum SettingsTab {
    Connection,
    Appearance,
    Plugins,
    Github,
    About,
}

struct OpenNote {
    id: Uuid,
    path: String,
    content: String,
    version: i32,
    /// Content as last persisted in the database
    saved: String,
    last_edit: Option<f64>,
    /// Content currently being saved
    saving: Option<String>,
    conflict: Option<Conflict>,
    remote_flash: f64,
}

#[derive(Default, Clone)]
struct Tab {
    /// the dashboard tab (projects / tickets) instead of a note
    dashboard: bool,
    note: Option<Uuid>,
    back: Vec<Uuid>,
    fwd: Vec<Uuid>,
}

#[derive(Clone, Copy, PartialEq)]
enum OpenMode {
    /// open in the current tab, pushing the previous note to its history
    Replace,
    NewTab,
    /// navigation (back / forward / tab switch): no history change
    History,
}

#[derive(Clone, Copy, PartialEq)]
enum SortOrder {
    NameAsc,
    NameDesc,
    Modified,
}

#[derive(PartialEq, Clone, Copy)]
enum ConflictTab {
    Resolve,
    Diff,
}

/// A divergence between the local editor and a newer version in the database,
/// resolved hunk by hunk (three-way merge, base = last known saved version).
struct Conflict {
    remote: Note,
    blocks: Vec<Block>,
    tab: ConflictTab,
    diff_against_result: bool,
    window_open: bool,
}

impl OpenNote {
    fn dirty(&self) -> bool {
        self.content != self.saved || self.saving.is_some()
    }
}

enum Dialog {
    NewNote { input: String, error: Option<String> },
    NewFolder { input: String, error: Option<String> },
    RenameNote { id: Uuid, input: String, error: Option<String> },
    RenameFolder { from: String, input: String, error: Option<String> },
    DeleteNote { id: Uuid, path: String },
    DeleteFolder { path: String },
    Switcher { query: String, sel: usize },
    Palette { query: String, sel: usize },
}

#[derive(Clone)]
enum UiAction {
    OpenId(Uuid),
    OpenLink(String),
    OpenTag(String),
    NewNoteIn(String),
    NewFolderIn(String),
    RenameNote(Uuid, String),
    RenameFolder(String),
    DeleteNote(Uuid, String),
    DeleteFolder(String),
    Core(CoreCmd),
    Plugin(usize, String),
    OpenNewTab(Uuid),
    SwitchTab(usize),
    CloseTab(usize),
    CloseOtherTabs(usize),
    ToggleFolder(String),
    SetSort(SortOrder),
    Back,
    Forward,
}

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

#[derive(Default)]
struct TreeFolder {
    name: String,
    path: String,
    folders: BTreeMap<String, TreeFolder>,
    notes: Vec<(String, Uuid, chrono::DateTime<chrono::Utc>)>,
}

struct Autocomplete {
    /// char index where the partial link starts (just after `[[`)
    start: usize,
    partial: String,
    sel: usize,
    matches: Vec<String>,
}

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

/// Obsidian-style CSS themes / snippets currently loaded.
#[derive(Default)]
struct CssState {
    themes: Vec<(String, std::path::PathBuf)>,
    snippets: Vec<(String, std::path::PathBuf)>,
    /// files read for the current override, with their modification time (hot reload)
    watched: Vec<(std::path::PathBuf, Option<std::time::SystemTime>)>,
    active_theme: Option<String>,
    active_snippets: Vec<String>,
    report: Vec<String>,
    last_check: f64,
    obsidian_input: String,
}

#[derive(Default)]
struct Needs {
    board: bool,
    notes: bool,
    folders: bool,
    backlinks: bool,
    graph: bool,
    tags: bool,
    plugins: bool,
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

fn char_to_byte(s: &str, ci: usize) -> usize {
    s.char_indices().nth(ci).map(|(b, _)| b).unwrap_or(s.len())
}

fn editor_id(note: Uuid) -> egui::Id {
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
                    self.need = Needs { notes: true, folders: true, plugins: true, tags: true, board: true, ..Default::default() };
                    self.send_github_config();
                }
                Msg::ConnectionFailed(e) => self.conn = ConnState::Disconnected(Some(e)),
                Msg::Live(v) => self.live = v,
                Msg::Resync => {
                    self.need = Needs { notes: true, folders: true, plugins: true, backlinks: true, tags: true, graph: self.graph.open, board: true };
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
                Msg::Loaded { note, purpose: LoadPurpose::Open } | Msg::Created { note, open: true } => self.set_open(note, now),
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
            n.conflict = Some(Conflict { remote, blocks, tab: ConflictTab::Resolve, diff_against_result: false, window_open: true });
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
                cur = cur
                    .folders
                    .entry(seg.to_lowercase())
                    .or_insert_with(|| TreeFolder { name: seg.to_string(), path: p, ..Default::default() });
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
        let current_folder = || self.open.as_ref().and_then(|n| parent(&n.path).map(|p| format!("{p}/"))).unwrap_or_default();
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
                self.cfg.view_mode = if self.cfg.view_mode == ViewMode::Preview { ViewMode::Edit } else { ViewMode::Preview };
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
                self.need = Needs { notes: true, folders: true, plugins: true, backlinks: true, tags: true, graph: self.graph.open, board: true };
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
        url.rsplit('/').next().and_then(|s| s.split('?').next()).filter(|s| !s.is_empty() && !s.contains('@')).unwrap_or("Mnemo").to_string()
    }

    fn left_sidebar(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.pal();
        // header: view tabs
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 40.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            for (tab, ic, tip) in [(LeftTab::Files, icon::FILES_TAB, "Fichiers"), (LeftTab::Search, icon::SEARCH, "Rechercher"), (LeftTab::Tags, icon::TAG, "Tags")] {
                if theme::icon_button_sized(ui, ic, tip, self.left_tab == tab, true, 16.0).clicked() {
                    self.left_tab = tab;
                    if tab == LeftTab::Search {
                        self.focus_search = true;
                    }
                }
            }
        });
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
                        if theme::icon_button_sized(ui, icon::NEW_FOLDER, "Nouveau dossier", false, true, 16.0).clicked() {
                            self.actions.push(UiAction::Core(CoreCmd::NewFolder));
                        }
                        let sort = theme::icon_button_sized(ui, icon::SORT, "Changer l'ordre de tri", false, true, 16.0);
                        egui::Popup::menu(&sort).show(|ui| {
                            for (o, label) in [(SortOrder::NameAsc, "Nom du fichier (A à Z)"), (SortOrder::NameDesc, "Nom du fichier (Z à A)"), (SortOrder::Modified, "Date de modification (récent d'abord)")] {
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
                        let rest = ui.allocate_response(ui.available_size().max(egui::vec2(10.0, 40.0)), egui::Sense::click());
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
                                if theme::tree_row(ui, basename(&hit.path), 0, open_id == Some(hit.id), Some(true)).clicked() {
                                    self.actions.push(UiAction::OpenId(hit.id));
                                }
                                egui::Frame::new().fill(p.bg).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
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
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), footer_h - 2.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            let name = self.vault_name();
            let r = ui.add(egui::Button::new(
                RichText::new(format!("{name}  ")).color(p.text).strong()).frame(false)
            ).on_hover_text(format!("Base : {}", redact_url(&self.cfg.database_url)));
            ui.label(theme::icon_text(icon::VAULT_SWITCH, 13.0).color(p.faint));
            if r.clicked() {
                self.settings_tab = SettingsTab::Connection;
                self.show_settings = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                if theme::icon_button_sized(ui, icon::SETTINGS, "Paramètres", self.show_settings, true, 16.0).clicked() {
                    self.actions.push(UiAction::Core(CoreCmd::Settings));
                }
                if theme::icon_button_sized(ui, icon::HELP, "Aide et raccourcis", false, true, 16.0).clicked() {
                    self.settings_tab = SettingsTab::About;
                    self.show_settings = true;
                }
            });
        });
    }

    fn right_sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let tabs: Vec<(&str, String)> = std::iter::once((icon::LINK, "Rétroliens".to_string()))
            .chain(std::iter::once((icon::TICKET, "Tickets liés".to_string())))
            .chain(self.plugin_panels.iter().map(|(name, _)| (if name == "Plan" { icon::LIST } else { icon::PUZZLE }, name.clone())))
            .collect();
        self.right_tab = self.right_tab.min(tabs.len() - 1);
        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 40.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add_space(6.0);
            for (i, (ic, name)) in tabs.iter().enumerate() {
                if theme::icon_button_sized(ui, ic, name, self.right_tab == i, true, 16.0).clicked() {
                    self.right_tab = i;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(4.0);
                if theme::icon_button_sized(ui, icon::PANEL_RIGHT, "Masquer le panneau de droite", false, true, 16.0).clicked() {
                    self.actions.push(UiAction::Core(CoreCmd::ToggleRight));
                }
            });
        });
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
                        if theme::tree_row(ui, basename(&b.path), 0, false, Some(true)).on_hover_text(&b.path).clicked() {
                            self.actions.push(UiAction::OpenId(b.id));
                        }
                        for s in &b.snippets {
                            egui::Frame::new().fill(p.bg).corner_radius(4.0).inner_margin(egui::Margin::symmetric(8, 5)).show(ui, |ui| {
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
                    CommonMarkViewer::new().show_with_id(egui::Id::unique(("plugin-panel", self.right_tab)), ui, &mut self.md_cache, &src);
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
                                    ui.label(RichText::new("Conflit à résoudre").size(12.0).color(Color32::from_rgb(0xe9, 0x97, 0x3f)));
                                }
                                ui.label(small(format!("{} rétrolien{}", self.backlinks.len(), if self.backlinks.len() > 1 { "s" } else { "" })));
                                for s in &self.plugin_status {
                                    ui.label(small(s.clone()));
                                }
                                let now = ui.input(|i| i.time);
                                if now - n.remote_flash < 3.0 {
                                    ui.label(RichText::new("mis à jour à distance").size(12.0).color(p.accent));
                                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
                                }
                            }
                            let (ic, color, tip) = match (&self.conn, self.live, self.open.as_ref().map(|n| n.dirty())) {
                                (ConnState::Connected, true, Some(true)) => (icon::SYNC, p.faint, "Enregistrement…".to_string()),
                                (ConnState::Connected, true, _) => (icon::SYNC, Color32::from_rgb(0x44, 0xcf, 0x6e), "Synchronisé en direct".to_string()),
                                (ConnState::Connected, false, _) => (icon::SYNC, Color32::from_rgb(0xe9, 0x97, 0x3f), "Connecté, flux live en reconnexion…".to_string()),
                                _ => (icon::OFFLINE, Color32::from_rgb(0xfb, 0x46, 0x4c), "Hors ligne".to_string()),
                            };
                            ui.label(theme::icon_text(ic, 13.0).color(color)).on_hover_text(format!("{tip}\n{}", redact_url(&self.cfg.database_url)));
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
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Max)));
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
                    && theme::icon_button_sized(ui, icon::PANEL_RIGHT, "Afficher le panneau de droite", false, true, 16.0).clicked()
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

        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::left_to_right(egui::Align::Center)));
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
                if ui.button(RichText::new("Supprimer le fichier").color(Color32::from_rgb(0xfb, 0x46, 0x4c))).clicked() {
                    self.actions.push(UiAction::DeleteNote(id, path.clone()));
                    ui.close();
                }
            });
            let reading = self.cfg.view_mode == ViewMode::Preview;
            let (ic, tip) = if reading { (icon::EDITING, "Basculer en mode édition (Ctrl+E)") } else { (icon::READING, "Basculer en mode lecture (Ctrl+E)") };
            if theme::icon_button_sized(ui, ic, tip, false, true, 16.0).clicked() {
                self.actions.push(UiAction::Core(CoreCmd::ToggleMode));
            }
            if self.cfg.view_mode == ViewMode::Split && theme::icon_button_sized(ui, icon::SPLIT, "Quitter le mode divisé", true, true, 16.0).clicked() {
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
            let link = |ui: &mut egui::Ui, t: &str| ui.add(egui::Label::new(RichText::new(t).size(15.0).color(p.link)).sense(egui::Sense::click()));
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

    // ------------------------------------------------------------------ CSS themes / snippets
    fn css_dirs(&self) -> Vec<std::path::PathBuf> {
        let mut dirs = vec![];
        if let Some(d) = &self.cfg.obsidian_dir {
            dirs.push(d.clone());
        }
        dirs.push(crate::config::config_dir());
        dirs
    }

    fn install_example_snippet(&self) {
        let dir = crate::config::config_dir().join("snippets");
        let file = dir.join("exemple-accent-vert.css");
        if !dir.exists() {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::create_dir_all(crate::config::config_dir().join("themes"));
            let _ = std::fs::write(&file, EXAMPLE_SNIPPET);
        }
    }

    /// (Re)reads themes, snippets and appearance.json and applies their CSS variables.
    fn reload_css(&mut self) {
        let dirs = self.css_dirs();
        let (themes, snippets) = crate::css::discover(&dirs);
        self.css.themes = themes;
        self.css.snippets = snippets;
        let mut watched: Vec<std::path::PathBuf> = vec![];
        let mut extra: Vec<(String, String)> = vec![];
        let mut report = vec![];
        let (theme_name, snippet_names) = match (&self.cfg.obsidian_dir, self.cfg.follow_appearance) {
            (Some(dir), true) => {
                watched.push(dir.join("appearance.json"));
                match crate::css::read_appearance(dir) {
                    Some(a) => {
                        if let Some(size) = a.base_font_size {
                            extra.push(("--font-text-size".into(), format!("{size}px")));
                        }
                        if let Some(acc) = &a.accent {
                            for k in ["--color-accent", "--interactive-accent", "--text-accent", "--checkbox-color", "--blockquote-border-color"] {
                                extra.push((k.into(), acc.clone()));
                            }
                        }
                        if let Some(dark) = a.dark
                            && dark != self.cfg.dark_mode
                        {
                            self.cfg.dark_mode = dark;
                            self.cfg.save();
                        }
                        report.push("appearance.json lu".to_string());
                        (a.css_theme, a.snippets)
                    }
                    None => {
                        report.push("appearance.json introuvable dans ce dossier".to_string());
                        (self.cfg.css_theme.clone(), self.cfg.css_snippets.clone())
                    }
                }
            }
            _ => (self.cfg.css_theme.clone(), self.cfg.css_snippets.clone()),
        };
        let mut parsed = crate::css::Parsed::default();
        let mut files = vec![];
        if let Some(name) = &theme_name {
            match self.css.themes.iter().find(|(n, _)| n == name) {
                Some((_, path)) => files.push((format!("thème « {name} »"), path.clone())),
                None => report.push(format!("Thème « {name} » introuvable (themes/{name}/theme.css)")),
            }
        }
        for name in &snippet_names {
            match self.css.snippets.iter().find(|(n, _)| n == name) {
                Some((_, path)) => files.push((format!("snippet « {name} »"), path.clone())),
                None => report.push(format!("Snippet « {name} » introuvable")),
            }
        }
        for (label, path) in &files {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    crate::css::parse(&text, &mut parsed);
                    report.push(format!("{label} chargé"));
                }
                Err(e) => report.push(format!("{label} : lecture impossible ({e})")),
            }
            watched.push(path.clone());
        }
        self.css.active_theme = theme_name;
        self.css.active_snippets = snippet_names;
        if parsed.vars.is_empty() && extra.is_empty() {
            theme::set_override(None);
        } else {
            let d = crate::css::apply(&parsed, &extra, true, theme::builtin_palette(true), theme::Metrics::default());
            let l = crate::css::apply(&parsed, &extra, false, theme::builtin_palette(false), theme::Metrics::default());
            let mut names: Vec<&str> = parsed.vars.iter().map(|v| v.1.as_str()).collect();
            names.sort();
            names.dedup();
            let known = names.iter().filter(|n| crate::css::is_known(n)).count();
            report.push(format!(
                "{} variable(s) lue(s) : {known} appliquée(s) directement, {} autre(s) utilisée(s) seulement via var()",
                names.len(),
                names.len() - known
            ));
            if parsed.ignored_rules > 0 {
                report.push(format!("{} règle(s) ciblant des éléments précis ignorée(s) (non applicables hors navigateur)", parsed.ignored_rules));
            }
            if names.iter().any(|n| n.starts_with("--font-") && !n.ends_with("-size")) {
                report.push("Polices personnalisées (--font-text / --font-interface) non prises en charge : Inter reste utilisée".into());
            }
            let _ = (d.used.len(), l.used.len());
            theme::set_override(Some(theme::Override { dark: (d.palette, d.metrics), light: (l.palette, l.metrics) }));
        }
        let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        self.css.watched = watched.into_iter().map(|p| { let t = mtime(&p); (p, t) }).collect();
        self.css.report = report;
        self.theme_applied = None;
        self.preview_gen = u64::MAX;
    }

    /// Hot reload: re-apply CSS when a watched file changes (checked once per second).
    fn watch_css(&mut self, ctx: &egui::Context, now: f64) {
        if now - self.css.last_check < 1.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
            return;
        }
        self.css.last_check = now;
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let changed = self
            .css
            .watched
            .iter()
            .any(|(p, t)| std::fs::metadata(p).and_then(|m| m.modified()).ok() != *t);
        if changed {
            self.reload_css();
            self.notice("Thème CSS rechargé", false, now);
        }
    }

    fn send_github_config(&self) {
        let cfg = self.cfg.github_token.clone().filter(|t| !t.trim().is_empty()).map(|token| crate::github::GhConfig {
            token,
            api: self.cfg.github_api.clone(),
        });
        self.db.send(Cmd::SetGithub(cfg));
    }

    fn github_settings(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        ui.add(egui::Label::new(RichText::new(
            "Reliez vos projets du tableau de bord à des dépôts GitHub : import des issues, publication des tickets, \
             synchronisation du titre, de la description, des labels (tags) et de l'état ouvert / fermé.",
        ).color(p.muted)).wrap());
        ui.add_space(10.0);
        ui.label(RichText::new("Token d'accès personnel").color(p.text));
        ui.add(egui::Label::new(RichText::new(
            "GitHub › Settings › Developer settings › Personal access tokens. Fine-grained : accès « Issues : lecture et écriture » \
             sur les dépôts voulus ; classique : portée « repo ». Le token est stocké uniquement dans le fichier de configuration local \
             (ou la variable MNEMO_GITHUB_TOKEN), jamais dans la base.",
        ).size(12.5).color(p.faint)).wrap());
        ui.horizontal(|ui| {
            let hint = if self.cfg.github_token.is_some() { "•••••••• (enregistré)" } else { "ghp_… ou github_pat_…" };
            ui.add(egui::TextEdit::singleline(&mut self.github_input).password(true).hint_text(hint).desired_width(320.0));
            if ui.button("Enregistrer").clicked() && !self.github_input.trim().is_empty() {
                self.cfg.github_token = Some(self.github_input.trim().to_string());
                self.github_input.clear();
                self.cfg.save();
                self.send_github_config();
                self.dash.github_user = None;
            }
            if self.cfg.github_token.is_some() && ui.button("Supprimer").clicked() {
                self.cfg.github_token = None;
                self.cfg.save();
                self.send_github_config();
                self.dash.github_user = None;
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("URL de l'API").color(p.text));
        let r = ui.add(egui::TextEdit::singleline(&mut self.cfg.github_api).desired_width(320.0));
        ui.label(RichText::new("https://api.github.com, ou https://<serveur>/api/v3 pour GitHub Enterprise").size(12.0).color(p.faint));
        if r.lost_focus() {
            self.cfg.save();
            self.send_github_config();
        }
        ui.add_space(8.0);
        if ui.checkbox(&mut self.cfg.github_auto_sync, "Synchroniser automatiquement toutes les 5 minutes (tableau de bord ouvert)").changed() {
            self.cfg.save();
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(self.cfg.github_token.is_some(), egui::Button::new("Tester la connexion")).clicked() {
                self.dash.github_user = None;
                self.db.send(Cmd::GithubTest);
            }
            match &self.dash.github_user {
                Some(Ok(login)) => {
                    ui.label(RichText::new(format!("Connecté en tant que {login}")).color(crate::board::Status::Done.color()));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(Color32::from_rgb(0xfb, 0x46, 0x4c)));
                }
                None => {}
            }
        });
        ui.add_space(10.0);
        let linked: Vec<String> = self.board.projects.iter().filter_map(|pr| pr.github_repo.as_ref().map(|r| format!("{} → {r}", pr.name))).collect();
        ui.label(RichText::new("Projets reliés").color(p.text));
        if linked.is_empty() {
            ui.label(RichText::new("Aucun : ouvrez le tableau de bord › Modifier le projet › Dépôt GitHub.").color(p.faint));
        }
        for l in linked {
            ui.label(RichText::new(format!("• {l}")).color(p.muted));
        }
    }

    fn appearance_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        let p = self.pal();
        let following = self.cfg.follow_appearance && self.cfg.obsidian_dir.is_some();
        let row = |ui: &mut egui::Ui, title: &str, desc: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 260.0).max(200.0));
                    ui.label(RichText::new(title).color(p.text));
                    if !desc.is_empty() {
                        ui.add(egui::Label::new(RichText::new(desc).size(12.5).color(p.muted)).wrap());
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| add(ui));
            });
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
            ui.painter().hline(r.x_range(), r.center().y, egui::Stroke::new(1.0, p.border));
            ui.add_space(8.0);
        };
        let mut changed = false;
        let mut dark = self.cfg.dark_mode;
        row(ui, "Thème de base", "Sombre ou clair.", &mut |ui| {
            egui::ComboBox::from_id_salt("base-theme").selected_text(if dark { "Sombre" } else { "Clair" }).show_ui(ui, |ui| {
                ui.selectable_value(&mut dark, true, "Sombre");
                ui.selectable_value(&mut dark, false, "Clair");
            });
        });
        if dark != self.cfg.dark_mode {
            self.cfg.dark_mode = dark;
            changed = true;
        }
        let mut size = self.cfg.font_size;
        row(ui, "Taille de police", "Taille du texte de l'éditeur et du mode lecture (remplacée par --font-text-size si un thème la définit).", &mut |ui| {
            ui.add(egui::Slider::new(&mut size, 11.0..=26.0).step_by(1.0).suffix(" px"));
        });
        if size != self.cfg.font_size {
            self.cfg.font_size = size;
            changed = true;
        }

        ui.add_space(10.0);
        ui.label(RichText::new("Thèmes et snippets CSS").size(16.0).strong().color(p.text));
        ui.add(egui::Label::new(RichText::new(
            "Mnemo lit les fichiers CSS au format Obsidian (themes/<Nom>/theme.css, snippets/*.css) et applique leurs variables CSS \
             (--background-primary, --text-normal, --interactive-accent, --font-text-size, --file-line-width…). Les fichiers sont rechargés à chaud.",
        ).size(12.5).color(p.muted)).wrap());
        ui.add_space(10.0);

        let mut input = self.css.obsidian_input.clone();
        let mut apply_dir = false;
        row(ui, "Dossier .obsidian d'un coffre", "Facultatif : réutilise les thèmes, snippets et appearance.json d'un coffre Obsidian existant.", &mut |ui| {
            if ui.button("Appliquer").clicked() {
                apply_dir = true;
            }
            ui.add(egui::TextEdit::singleline(&mut input).hint_text("~/MonCoffre/.obsidian").desired_width(220.0));
        });
        self.css.obsidian_input = input;
        if apply_dir {
            let raw = self.css.obsidian_input.trim();
            let expanded = if let Some(rest) = raw.strip_prefix("~/") {
                directories::BaseDirs::new().map(|b| b.home_dir().join(rest)).unwrap_or_else(|| raw.into())
            } else {
                std::path::PathBuf::from(raw)
            };
            if raw.is_empty() {
                self.cfg.obsidian_dir = None;
            } else if expanded.is_dir() {
                // accept the vault folder itself too
                let dir = if expanded.join(".obsidian").is_dir() { expanded.join(".obsidian") } else { expanded };
                self.css.obsidian_input = dir.display().to_string();
                self.cfg.obsidian_dir = Some(dir);
            } else {
                self.notice(format!("Dossier introuvable : {}", expanded.display()), true, now);
            }
            changed = true;
        }
        if self.cfg.obsidian_dir.is_some() {
            let mut follow = self.cfg.follow_appearance;
            row(ui, "Suivre appearance.json", "Utilise le thème, les snippets activés, la taille de police, la couleur d'accent et le mode clair/sombre de ce coffre.", &mut |ui| {
                ui.checkbox(&mut follow, "");
            });
            if follow != self.cfg.follow_appearance {
                self.cfg.follow_appearance = follow;
                changed = true;
            }
        }

        let mut theme_sel = self.css.active_theme.clone();
        let names: Vec<String> = self.css.themes.iter().map(|t| t.0.clone()).collect();
        row(ui, "Thème", if following { "Défini par appearance.json." } else { "Thèmes trouvés dans les dossiers themes/." }, &mut |ui| {
            ui.add_enabled_ui(!following, |ui| {
                egui::ComboBox::from_id_salt("css-theme").selected_text(theme_sel.clone().unwrap_or_else(|| "Par défaut".into())).show_ui(ui, |ui| {
                    ui.selectable_value(&mut theme_sel, None, "Par défaut");
                    for n in &names {
                        ui.selectable_value(&mut theme_sel, Some(n.clone()), n);
                    }
                });
            });
        });
        if !following && theme_sel != self.css.active_theme {
            self.cfg.css_theme = theme_sel;
            changed = true;
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Snippets CSS").color(p.text));
        ui.add_space(4.0);
        if self.css.snippets.is_empty() {
            ui.label(RichText::new("Aucun snippet trouvé.").color(p.faint));
        }
        let snippets = self.css.snippets.clone();
        for (name, path) in &snippets {
            let mut on = self.css.active_snippets.contains(name);
            let before = on;
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!following, |ui| ui.checkbox(&mut on, ""));
                ui.label(RichText::new(name).color(p.text));
                ui.add(egui::Label::new(RichText::new(path.display().to_string()).size(11.5).color(p.faint)).truncate())
                    .on_hover_text(path.display().to_string());
            });
            if on != before && !following {
                let mut list = self.css.active_snippets.clone();
                if on {
                    list.push(name.clone());
                } else {
                    list.retain(|n| n != name);
                }
                self.cfg.css_snippets = list;
                changed = true;
            }
        }
        ui.add_space(8.0);
        if ui.button("Recharger").clicked() {
            changed = true;
        }
        ui.add_space(4.0);
        for d in self.css_dirs() {
            ui.add(egui::Label::new(RichText::new(format!("Dossier lu : {}", d.display())).size(11.5).color(p.faint)).truncate())
                .on_hover_text(d.display().to_string());
        }
        ui.add_space(8.0);
        egui::Frame::new().fill(p.bg2).corner_radius(6.0).inner_margin(10.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Rapport").size(12.5).strong().color(p.muted));
            if self.css.report.is_empty() {
                ui.label(RichText::new("Thème par défaut, aucun CSS chargé.").size(12.5).color(p.faint));
            }
            for line in &self.css.report {
                ui.add(egui::Label::new(RichText::new(format!("• {line}")).size(12.5).color(p.muted)).wrap());
            }
        });
        if changed {
            self.cfg.save();
            self.reload_css();
        }
    }

    fn inline_title(&mut self, ui: &mut egui::Ui, editable: bool) {
        let p = self.pal();
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

    fn editor_area(&mut self, ui: &mut egui::Ui, now: f64) {
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
                    egui::ScrollArea::vertical().id_salt("split-edit").auto_shrink([false, false]).show(&mut cols[0], |ui| {
                        centered(ui, width, |ui| {
                            ui.add_space(26.0);
                            self.inline_title(ui, true);
                            ui.add_space(14.0);
                            self.editor(ui, note_id, now);
                            ui.add_space(120.0);
                        });
                    });
                    egui::ScrollArea::vertical().id_salt("split-read").auto_shrink([false, false]).show(&mut cols[1], |ui| {
                        centered(ui, width, |ui| {
                            ui.add_space(26.0);
                            self.inline_title(ui, false);
                            ui.add_space(14.0);
                            self.preview(ui, now);
                            ui.add_space(120.0);
                        });
                    });
                });
            }
        }
    }

    fn editor(&mut self, ui: &mut egui::Ui, note_id: Uuid, now: f64) {
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

        let pal = self.pal();
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
            theme::decorate_editor(&n.content, active, &out.galley, out.galley_pos, right, &pal, &met, ui.painter(), bg_slot)
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
                    let show_create = !partial.trim().is_empty() && !matches.iter().any(|m| m.eq_ignore_ascii_case(partial.trim()));
                    if show_create {
                        matches.push(partial.trim().to_string());
                    }
                    let prev_sel = self.prev_ac_sel.unwrap_or(0);
                    let mut ac = Autocomplete { start, partial: partial.to_string(), sel: 0, matches };
                    ac.sel = prev_sel.min(ac.matches.len().saturating_sub(1));
                    if !ac.matches.is_empty() {
                        let mut clicked = None;
                        egui::Area::new(egui::Id::unique("wikilink-ac")).order(egui::Order::Foreground).fixed_pos(pos + egui::vec2(0.0, 6.0)).show(
                            ui.ctx(),
                            |ui| {
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
                                    ui.label(RichText::new("↑↓ naviguer   ↵ insérer   échap ignorer").size(11.5).color(pal.faint));
                                });
                            },
                        );
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

    fn accept_autocomplete(&mut self, ctx: &egui::Context, target: &str, now: f64) {
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

    fn preview(&mut self, ui: &mut egui::Ui, now: f64) {
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
            CommonMarkViewer::new().default_width(Some(ui.available_width() as usize)).show_mut(ui, &mut self.md_cache, &mut copy);
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

    fn dialogs(&mut self, ctx: &egui::Context, now: f64) {
        let Some(mut dialog) = self.dialog.take() else { return };
        let mut close = false;
        let focus = std::mem::take(&mut self.focus_dialog);
        let prompt = matches!(dialog, Dialog::Switcher { .. } | Dialog::Palette { .. });
        let p = self.pal();
        let mid = egui::Id::unique("mnemo-dialog");
        let mut modal = egui::Modal::new(mid).backdrop_color(Color32::from_black_alpha(if self.cfg.dark_mode { 110 } else { 50 })).frame(
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
                    let count = self.notes.iter().filter(|n| n.path.to_lowercase().starts_with(&(path.to_lowercase() + "/"))).count();
                    ui.heading("Supprimer le dossier ?");
                    ui.label(format!("« {path} » et ses {count} note(s) seront supprimés définitivement."));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Supprimer").color(Color32::from_rgb(230, 90, 90))).clicked() {
                            if self.open.as_ref().is_some_and(|n| n.path.to_lowercase().starts_with(&(path.to_lowercase() + "/"))) {
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
                    let mut items: Vec<&NoteMeta> = self.notes.iter().filter(|n| words.iter().all(|w| fold(&n.path).contains(w))).collect();
                    items.sort_by_key(|n| (!fold(basename(&n.path)).starts_with(&q), std::cmp::Reverse(n.updated_at)));
                    items.truncate(12);
                    let create = !query.trim().is_empty() && !items.iter().any(|n| n.path.eq_ignore_ascii_case(query.trim()));
                    nav_keys(ui, sel, items.len() + usize::from(create));
                    let r = prompt_field(ui, query, "Trouver ou créer une note…");
                    if r.changed() {
                        *sel = 0;
                    }
                    let mut chosen: Option<Option<Uuid>> = None;
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(6, 0)).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        for (i, n) in items.iter().enumerate() {
                            let r = theme::suggestion_row(ui, basename(&n.path), parent(&n.path).unwrap_or(""), *sel == i);
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
                    prompt_footer(ui, &p, &[("↑↓", "pour naviguer"), ("↵", "pour ouvrir"), ("ctrl ↵", "dans un nouvel onglet"), ("échap", "pour annuler")]);
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
                        .chain(self.plugin_cmds.iter().map(|(i, id, name)| (name.clone(), String::new(), UiAction::Plugin(*i, id.clone()))))
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
                    prompt_footer(ui, &p, &[("↑↓", "pour naviguer"), ("↵", "pour utiliser"), ("échap", "pour annuler")]);
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

    fn settings_window(&mut self, ctx: &egui::Context, now: f64) {
        let p = self.pal();
        let mut open = self.show_settings;
        let screen = ctx.content_rect();
        let size = egui::vec2((screen.width() - 80.0).clamp(500.0, 1000.0), (screen.height() - 90.0).clamp(360.0, 720.0));
        let modal = egui::Modal::new(egui::Id::unique("settings-modal"))
            .backdrop_color(Color32::from_black_alpha(if self.cfg.dark_mode { 110 } else { 50 }))
            .frame(egui::Frame::new().fill(p.bg).stroke(egui::Stroke::new(1.0, p.border)).corner_radius(10.0).shadow(ctx.global_style().visuals.window_shadow));
        let resp = modal.show(ctx, |ui| {
            ui.set_min_size(size);
            ui.set_max_size(size);
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // left navigation
                egui::Frame::new().fill(p.bg2).corner_radius(egui::CornerRadius { nw: 10, sw: 10, ne: 0, se: 0 }).inner_margin(egui::Margin::symmetric(10, 14)).show(ui, |ui| {
                    ui.set_width(200.0);
                    ui.set_min_height(size.y - 28.0);
                    ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new("Options").size(12.0).color(p.faint).strong());
                    ui.add_space(4.0);
                    for (t, label) in [(SettingsTab::Connection, "Base de données"), (SettingsTab::Appearance, "Apparence"), (SettingsTab::Github, "GitHub"), (SettingsTab::About, "À propos")] {
                        if theme::tree_row(ui, label, 0, self.settings_tab == t, None).clicked() {
                            self.settings_tab = t;
                        }
                    }
                    ui.add_space(14.0);
                    ui.label(RichText::new("Plugins").size(12.0).color(p.faint).strong());
                    ui.add_space(4.0);
                    if theme::tree_row(ui, "Plugins communautaires", 0, self.settings_tab == SettingsTab::Plugins, None).clicked() {
                        self.settings_tab = SettingsTab::Plugins;
                    }
                    });
                });
                // content
                ui.vertical(|ui| {
                    ui.set_width(size.x - 220.0);
                    ui.set_max_width(size.x - 220.0);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 34.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_space(8.0);
                        if theme::icon_button_sized(ui, "\u{e1b2}", "Fermer", false, true, 16.0).clicked() {
                            open = false;
                        }
                    });
                    egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(size.y - 44.0).show(ui, |ui| {
                        egui::Frame::new().inner_margin(egui::Margin { left: 28, right: 28, top: 0, bottom: 20 }).show(ui, |ui| {
                            ui.set_max_width(size.x - 220.0 - 56.0);
                            let title = match self.settings_tab {
                                SettingsTab::Connection => "Base de données",
                                SettingsTab::Appearance => "Apparence",
                                SettingsTab::Plugins => "Plugins",
                                SettingsTab::Github => "GitHub",
                                SettingsTab::About => "À propos",
                            };
                            ui.label(RichText::new(title).size(20.0).strong().color(p.text));
                            ui.add_space(12.0);
                            self.settings_content(ui, now);
                        });
                    });
                });
            });
        });
        if resp.should_close() {
            open = false;
        }
        self.show_settings = open;
    }

    fn settings_content(&mut self, ui: &mut egui::Ui, now: f64) {
        {
            match self.settings_tab {
                SettingsTab::Connection => {
                    ui.label("URL PostgreSQL");
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.url_input).password(!self.show_password).desired_width(420.0));
                        ui.checkbox(&mut self.show_password, "Afficher");
                    });
                    if ui.button("Se (re)connecter").clicked() {
                        self.connect();
                    }
                    ui.add_space(8.0);
                    ui.label(format!("Connecté à : {}", redact_url(&self.cfg.database_url)));
                    ui.label(format!("Live (LISTEN/NOTIFY) : {}", if self.live { "actif" } else { "inactif" }));
                    ui.weak(format!("Identifiant de cette instance : {}", self.client_id));
                    ui.add_space(8.0);
                    ui.weak(format!("Fichier de configuration : {}", crate::config::config_dir().join("config.toml").display()));
                    ui.weak("La variable d'environnement MNEMO_DATABASE_URL a priorité sur ce réglage.");
                }
                SettingsTab::Appearance => self.appearance_settings(ui, now),
                SettingsTab::Plugins => self.plugins_settings(ui, now),
                SettingsTab::Github => self.github_settings(ui),
                SettingsTab::About => {
                    ui.heading("Mnemo");
                    ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                    ui.label("Base de connaissances Markdown native (Rust + egui), stockée dans PostgreSQL.");
                    ui.label("Synchronisation temps réel via LISTEN/NOTIFY, plugins en Rhai.");
                    ui.add_space(8.0);
                    ui.label(RichText::new("Raccourcis").strong());
                    for (_, name, key) in CORE_COMMANDS.iter().filter(|c| !c.2.is_empty()) {
                        ui.label(format!("{key:>14}   {name}"));
                    }
                    ui.label(format!("{:>14}   Palette de commandes", "Ctrl+P"));
                }
            }
        }
    }

    fn plugins_settings(&mut self, ui: &mut egui::Ui, now: f64) {
        ui.horizontal(|ui| {
            if ui.button("⟳ Recharger").clicked() {
                self.actions.push(UiAction::Core(CoreCmd::ReloadPlugins));
            }
            if ui.button("Réinstaller les plugins d'exemple").clicked() {
                match self.plugins.install_bundled(true) {
                    Ok(()) => self.actions.push(UiAction::Core(CoreCmd::ReloadPlugins)),
                    Err(e) => self.notice(format!("Erreur : {e}"), true, now),
                }
            }
        });
        ui.weak(format!("Dossier des plugins : {}", self.plugins.dir.display()));
        ui.weak("L'activation et les réglages sont stockés dans la base : ils sont synchronisés entre vos appareils.");
        ui.add_space(6.0);
        let mut to_save: Vec<(String, PluginState)> = vec![];
        let mut retry = None;
        for (i, p) in self.plugins.plugins.iter().enumerate() {
            let m = &p.manifest;
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let mut enabled = p.enabled;
                    if ui.checkbox(&mut enabled, RichText::new(&m.name).strong()).changed() {
                        let mut st = self.plugins.state_of(&m.id);
                        st.enabled = enabled;
                        to_save.push((m.id.clone(), st));
                    }
                    ui.weak(format!("v{}  ·  {}{}", m.version, m.id, if m.author.is_empty() { String::new() } else { format!("  ·  par {}", m.author) }));
                });
                if !m.description.is_empty() {
                    ui.label(&m.description);
                }
                if let Some(e) = &p.error {
                    ui.colored_label(Color32::from_rgb(230, 90, 90), e);
                    if ui.small_button("Réessayer").clicked() {
                        retry = Some(i);
                    }
                }
                if !m.settings.is_empty() {
                    egui::CollapsingHeader::new("Réglages").id_salt(("plugin-settings", &m.id)).show(ui, |ui| {
                        let stored = self.plugins.state_of(&m.id);
                        let edit = self.plugin_edit.entry(m.id.clone()).or_insert_with(|| {
                            let mut map = serde_json::Map::new();
                            for d in &m.settings {
                                let v = stored
                                    .settings
                                    .get(&d.key)
                                    .cloned()
                                    .or_else(|| d.default.as_ref().map(toml_to_json))
                                    .unwrap_or(serde_json::Value::String(String::new()));
                                map.insert(d.key.clone(), v);
                            }
                            map
                        });
                        for d in &m.settings {
                            let v = edit.entry(d.key.clone()).or_insert(serde_json::Value::Null);
                            ui.label(&d.label);
                            match v {
                                serde_json::Value::Bool(b) => {
                                    ui.checkbox(b, "");
                                }
                                serde_json::Value::Number(num) => {
                                    let mut x = num.as_f64().unwrap_or(0.0);
                                    if ui.add(egui::DragValue::new(&mut x)).changed() {
                                        *v = if x.fract() == 0.0 { serde_json::json!(x as i64) } else { serde_json::json!(x) };
                                    }
                                }
                                other => {
                                    let mut s = other.as_str().map(str::to_string).unwrap_or_default();
                                    let multiline = d.kind.as_deref() == Some("multiline");
                                    let r = if multiline {
                                        ui.add(egui::TextEdit::multiline(&mut s).desired_rows(4).desired_width(f32::INFINITY).code_editor())
                                    } else {
                                        ui.add(egui::TextEdit::singleline(&mut s).desired_width(f32::INFINITY))
                                    };
                                    if r.changed() {
                                        *other = serde_json::Value::String(s);
                                    }
                                }
                            }
                        }
                        if ui.button("Enregistrer les réglages").clicked() {
                            let mut st = stored.clone();
                            st.enabled = p.enabled;
                            st.settings = edit.clone();
                            to_save.push((m.id.clone(), st));
                        }
                    });
                }
            });
            ui.add_space(4.0);
        }
        if self.plugins.plugins.is_empty() {
            ui.weak("Aucun plugin trouvé dans le dossier.");
        }
        for (id, st) in to_save {
            // optimistic local apply, then persist (the DB broadcasts to other devices)
            let mut states: HashMap<String, PluginState> =
                self.plugins.plugins.iter().map(|p| (p.manifest.id.clone(), self.plugins.state_of(&p.manifest.id))).collect();
            states.insert(id.clone(), st.clone());
            self.plugins.apply_states(states);
            self.plugin_changed();
            self.db.send(Cmd::SetPlugin { id, state: st });
        }
        if let Some(i) = retry {
            self.plugins.clear_error(i);
            self.plugin_changed();
        }
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
    let normal = TextFormat { font_id: FontId::proportional(ui.style().text_styles[&egui::TextStyle::Small].size + 1.0), color: ui.visuals().weak_text_color(), ..Default::default() };
    let hl = TextFormat { color: ui.visuals().strong_text_color(), background: ui.visuals().selection.bg_fill.gamma_multiply(0.45), ..normal.clone() };
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
        egui::Panel::left("ribbon").exact_size(44.0).resizable(false).frame(egui::Frame::new().fill(p.bg2)).show(ui, |ui| self.ribbon(ui));
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

const DEL_BG: Color32 = Color32::from_rgba_premultiplied(60, 16, 16, 90);
const INS_BG: Color32 = Color32::from_rgba_premultiplied(14, 52, 24, 90);
const THEIRS_BG: Color32 = Color32::from_rgba_premultiplied(16, 36, 66, 90);

fn code_block(ui: &mut egui::Ui, text: &str, bg: Option<Color32>) {
    let frame = egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 4)).corner_radius(4.0).fill(bg.unwrap_or(ui.visuals().extreme_bg_color));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        if text.is_empty() {
            ui.label(RichText::new("(vide : supprimé de ce côté)").italics().weak());
        } else {
            ui.add(egui::Label::new(RichText::new(text.trim_end_matches('\n')).monospace()).wrap());
        }
    });
}

fn resolve_ui(ui: &mut egui::Ui, blocks: &mut [Block]) {
    let mut k = 0;
    for b in blocks.iter_mut() {
        match b {
            Block::Text { text, origin } => {
                let lines: Vec<&str> = text.lines().collect();
                match origin {
                    Origin::Unchanged => {
                        if lines.len() <= 6 {
                            code_block(ui, text, None);
                        } else {
                            code_block(ui, &lines[..2].join("\n"), None);
                            ui.weak(format!("     ⋯ {} lignes identiques ⋯", lines.len() - 4));
                            code_block(ui, &lines[lines.len() - 2..].join("\n"), None);
                        }
                    }
                    o => {
                        let (label, bg) = match o {
                            Origin::Mine => ("Modifié par vous : fusionné automatiquement", INS_BG),
                            Origin::Theirs => ("Modifié à distance : fusionné automatiquement", THEIRS_BG),
                            _ => ("Modification identique des deux côtés", INS_BG),
                        };
                        ui.label(RichText::new(label).small().strong());
                        code_block(ui, text, Some(bg));
                    }
                }
            }
            Block::Conflict { base, mine, theirs, choice, custom } => {
                k += 1;
                let resolved = *choice != Choice::Unresolved;
                let stroke = if resolved { Color32::from_rgb(70, 170, 100) } else { Color32::from_rgb(230, 140, 40) };
                egui::Frame::new().stroke(egui::Stroke::new(1.5, stroke)).corner_radius(6.0).inner_margin(8.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Conflit {k}")).strong());
                        ui.label(RichText::new(if resolved { "✔ résolu" } else { "à résoudre" }).color(stroke).small());
                    });
                    ui.columns(2, |cols| {
                        cols[0].label(RichText::new("Votre version").strong().color(Color32::from_rgb(90, 200, 120)));
                        code_block(&mut cols[0], mine, Some(INS_BG));
                        cols[1].label(RichText::new("Version distante").strong().color(Color32::from_rgb(110, 160, 240)));
                        code_block(&mut cols[1], theirs, Some(THEIRS_BG));
                    });
                    if !base.is_empty() {
                        egui::CollapsingHeader::new(RichText::new("Version d'origine (avant les deux modifications)").small())
                            .id_salt(("base", k))
                            .show(ui, |ui| code_block(ui, base, None));
                    }
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(choice, Choice::Mine, "Garder la mienne");
                        ui.selectable_value(choice, Choice::Theirs, "Garder la distante");
                        ui.selectable_value(choice, Choice::MineThenTheirs, "Les deux (la mienne d'abord)");
                        ui.selectable_value(choice, Choice::TheirsThenMine, "Les deux (distante d'abord)");
                        if ui.selectable_label(*choice == Choice::Custom, "Éditer à la main…").clicked() && *choice != Choice::Custom {
                            *custom = merge::resolved_text(mine, theirs, choice, custom).unwrap_or_else(|| merge::concat(mine, theirs));
                            *choice = Choice::Custom;
                        }
                        if resolved && ui.small_button("Annuler le choix").clicked() {
                            *choice = Choice::Unresolved;
                        }
                    });
                    if *choice == Choice::Custom {
                        ui.add(egui::TextEdit::multiline(custom).code_editor().desired_rows(4).desired_width(f32::INFINITY));
                    } else if let Some(r) = merge::resolved_text(mine, theirs, choice, custom) {
                        ui.label(RichText::new("Résultat :").small());
                        code_block(ui, &r, None);
                    }
                });
            }
        }
        ui.add_space(4.0);
    }
}

fn diff_ui(ui: &mut egui::Ui, old: &str, new: &str, new_label: &str) {
    let hunks = merge::unified(old, new, 3);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("− version distante").monospace().background_color(DEL_BG));
        ui.label(RichText::new(format!("+ {new_label}")).monospace().background_color(INS_BG));
    });
    ui.add_space(4.0);
    if hunks.is_empty() {
        ui.weak("Aucune différence.");
        return;
    }
    for hunk in hunks {
        egui::Frame::group(ui.style()).inner_margin(0.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let first = &hunk[0];
            ui.label(
                RichText::new(format!(" @@ -{} +{} @@", first.old_no.unwrap_or(0), first.new_no.unwrap_or(0))).monospace().weak(),
            );
            for line in &hunk {
                let (bg, sign) = match line.tag {
                    similar::ChangeTag::Delete => (DEL_BG, "-"),
                    similar::ChangeTag::Insert => (INS_BG, "+"),
                    similar::ChangeTag::Equal => (Color32::TRANSPARENT, " "),
                };
                egui::Frame::new().fill(bg).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let num = |n: Option<usize>| n.map(|n| format!("{n:>4}")).unwrap_or_else(|| "    ".into());
                        ui.label(RichText::new(format!("{} {} {sign} ", num(line.old_no), num(line.new_no))).monospace().weak());
                        ui.add(egui::Label::new(RichText::new(&line.text).monospace()).wrap());
                    });
                });
            }
        });
        ui.add_space(6.0);
    }
}

impl MnemoApp {
    fn conflict_window(&mut self, ctx: &egui::Context, now: f64) {
        let Some(n) = self.open.as_mut() else { return };
        let path = n.path.clone();
        let mine_full = n.content.clone();
        let Some(c) = n.conflict.as_mut() else { return };
        if !c.window_open {
            return;
        }
        let mut open = true;
        let mut apply: Option<String> = None;
        let mut close = false;
        egui::Window::new("Résoudre le conflit").open(&mut open).default_size([980.0, 640.0]).collapsible(false).show(ctx, |ui| {
            let (total, left) = merge::conflict_count(&c.blocks);
            ui.horizontal(|ui| {
                ui.selectable_value(&mut c.tab, ConflictTab::Resolve, format!("Résolution ({}/{total})", total - left));
                ui.selectable_value(&mut c.tab, ConflictTab::Diff, "Diff complet");
            });
            ui.weak(format!(
                "« {path} » a été modifiée ailleurs (version {}) pendant que vous l'éditiez. \
                 Les changements qui ne se chevauchent pas sont déjà fusionnés ; choisissez pour chaque conflit.",
                c.remote.version
            ));
            ui.separator();
            let h = (ui.available_height() - 44.0).max(160.0);
            egui::ScrollArea::vertical().max_height(h).auto_shrink([false, false]).show(ui, |ui| match c.tab {
                ConflictTab::Resolve => resolve_ui(ui, &mut c.blocks),
                ConflictTab::Diff => {
                    ui.checkbox(&mut c.diff_against_result, "Comparer au résultat de la fusion (au lieu de votre version)");
                    let merged = merge::assemble(&c.blocks);
                    let (new, label) = match (&merged, c.diff_against_result) {
                        (Some(m), true) => (m.clone(), "résultat de la fusion"),
                        _ => (mine_full.clone(), "votre version"),
                    };
                    diff_ui(ui, &c.remote.content, &new, label);
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                let merged = merge::assemble(&c.blocks);
                let btn = egui::Button::new(RichText::new("✔ Appliquer la fusion").strong());
                if ui.add_enabled(merged.is_some(), btn).on_disabled_hover_text(format!("{left} conflit(s) restant(s)")).clicked() {
                    apply = merged;
                }
                let set_all = |c: &mut Conflict, ch: Choice| {
                    for b in &mut c.blocks {
                        if let Block::Conflict { choice, .. } = b {
                            *choice = ch.clone();
                        }
                    }
                };
                if ui.button("Tout : la mienne").clicked() {
                    set_all(c, Choice::Mine);
                }
                if ui.button("Tout : la distante").clicked() {
                    set_all(c, Choice::Theirs);
                }
                if ui.button("Résoudre plus tard").clicked() {
                    close = true;
                }
            });
        });
        if !open || close {
            c.window_open = false;
        }
        if let Some(text) = apply {
            let remote = n.conflict.take().unwrap().remote;
            n.content = text;
            n.saved = remote.content;
            n.version = remote.version;
            n.last_edit = Some(now - AUTOSAVE_DELAY);
            self.content_gen += 1;
            self.notice("Fusion appliquée et enregistrée", false, now);
        }
    }

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

const EXAMPLE_SNIPPET: &str = r#"/* Exemple de snippet CSS au format Obsidian.
   Activez-le dans Paramètres > Apparence > Snippets CSS ; il est rechargé à chaud. */
.theme-dark,
.theme-light {
  --accent-h: 152;
  --accent-s: 55%;
  --accent-l: 45%;
}
.theme-dark {
  --background-primary: #191b1d;
  --background-secondary: #212426;
}
body {
  --file-line-width: 760px;
}
"#;

const WELCOME: &str = r#"Mnemo est une base de connaissances **Markdown** stockée dans **PostgreSQL**.
Ouvrez Mnemo sur un autre ordinateur connecté à la même base : chaque frappe s'y affiche en direct.

## Les bases
- Créez une note avec **Ctrl+N** (utilisez `Dossier/Nom` pour ranger)
- Liez des notes avec des doubles crochets : [[Idées]] (tapez `[[` pour l'autocomplétion)
- Taguez avec #exemple et retrouvez vos tags dans l'onglet *Tags*
- **Ctrl+P** ouvre la palette de commandes, **Ctrl+O** le sélecteur de notes
- **Ctrl+G** affiche le graphe des liens

## Tâches
- [x] Installer Mnemo
- [ ] Activer des plugins (⚙ Paramètres > Plugins)
- [ ] Écrire ma première note

> [!tip] Plugins
> Activez le plugin « Encadrés » pour que ce bloc s'affiche joliment.

```rust
fn main() {
    println!("Le code est coloré !");
}
```
"#;
