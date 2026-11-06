mod appearance;
mod commands;
mod conflict;
mod dashboard;
mod dialogs;
mod editor;
mod settings;
mod sidebar;
mod state;

use commands::{CORE_COMMANDS, CoreCmd};
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

    fn note_ctx(&self) -> Option<(String, String, String)> {
        self.open.as_ref().map(|n| (n.id.to_string(), n.path.clone(), n.content.clone()))
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
