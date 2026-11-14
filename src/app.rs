mod appearance;
mod attachments;
mod commands;
mod conflict;
mod dashboard;
mod dialogs;
mod editor;
mod icons;
mod layout;
mod notes;
mod settings;
mod sidebar;
mod state;
mod sync;
mod vault;
mod whiteboard;

use commands::{CORE_COMMANDS, CoreCmd};
use editor::{char_to_byte, editor_id};
use notes::AUTOSAVE_DELAY;
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
    vault_input: String,
    /// icônes de l'explorateur : chemin en minuscules → icône Lucide ou emoji
    icons: HashMap<String, String>,
    icon_pick: Option<icons::IconPick>,
    /// pièces jointes du dossier (chemins relatifs avec extension)
    files: Vec<String>,
    /// whiteboards ouverts (chemin relatif → dessin et état de la vue)
    boards: HashMap<String, whiteboard::BoardView>,
    /// aperçus des whiteboards intégrés dans les notes : chemin → (date du fichier, dessin)
    sketch_cache: HashMap<String, (Option<std::time::SystemTime>, crate::sketch::Sketch)>,
    /// image ouverte depuis l'explorateur
    image_view: Option<String>,
    /// une pièce jointe a changé : oublier les images en cache
    forget_images: bool,
    /// donne le focus à cet éditeur à la prochaine frame (note qui vient d'être créée)
    focus_editor: Option<egui::Id>,
    /// dossier de l'explorateur survolé pendant un glisser depuis le système
    os_drop_target: Option<(String, f64)>,
    /// la coupure de la base a déjà été signalée (une seule alerte jusqu'au retour)
    offline_noticed: bool,
    /// le dossier des notes est indexé
    local_ready: bool,
    /// dossier des notes créé à ce lancement (note de bienvenue)
    fresh_vault: bool,
    vault_last: Option<(crate::db::VaultStats, chrono::DateTime<chrono::Local>)>,
    /// le texte de la note ouverte vient d'être remplacé (autre appareil, fusion) : note et texte d'avant,
    /// pour remettre le curseur sur sa ligne
    cursor_fix: Option<(Uuid, String)>,
    /// dernier enregistrement envoyé (note, chemin, texte) : s'il est refusé alors que la note n'est plus
    /// ouverte, ce texte est gardé dans une note à part
    sent_save: Option<(Uuid, String, String)>,
    /// heure de la frame en cours
    now: f64,
    /// pièces jointes en cours d'enregistrement pour une note : (chemin demandé, note où l'intégrer)
    embed_targets: Vec<(String, Uuid)>,
}

/// Lowercase and strip French accents, for fuzzy matching ("theme" finds "Thème").
fn fold_accents(s: &str) -> String {
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
        // images des notes (file://) : PNG, JPEG, GIF, WebP, BMP, SVG
        egui_extras::install_image_loaders(&cc.egui_ctx);
        let client_id = Uuid::new_v4().to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = cc.egui_ctx.clone();
        let shared = crate::local::shared::new_shared();
        let db = crate::db::spawn(client_id.clone(), tx, move || ctx.request_repaint(), shared);
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
            vault_input: String::new(),
            icons: HashMap::new(),
            icon_pick: None,
            files: vec![],
            image_view: None,
            boards: HashMap::new(),
            sketch_cache: HashMap::new(),
            forget_images: false,
            focus_editor: None,
            os_drop_target: None,
            offline_noticed: false,
            local_ready: false,
            fresh_vault: false,
            vault_last: None,
            cursor_fix: None,
            sent_save: None,
            now: 0.0,
            embed_targets: vec![],
            cfg,
        };
        if let Some(e) = app.cfg.load_error.take() {
            app.notices.push((e, 20.0, true));
        }
        app.css.obsidian_input = app.cfg.obsidian_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        app.vault_input = app.cfg.vault_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
        app.install_example_snippet();
        app.reload_css();
        // le dossier des notes est la référence : Mnemo fonctionne sans base de données
        let dir = app.cfg.vault_dir.clone().unwrap_or_else(MnemoApp::default_notes_dir);
        app.fresh_vault = !dir.exists() || std::fs::read_dir(&dir).map(|mut d| d.next().is_none()).unwrap_or(true);
        if app.cfg.vault_dir.is_none() {
            app.cfg.vault_dir = Some(dir.clone());
            app.cfg.save();
        }
        app.vault_input = dir.display().to_string();
        app.db.send(Cmd::SetRoot(dir.clone()));
        // le worker connaît le dossier dès le départ : la synchro ne dépend pas de l'affichage
        // (une fenêtre cachée ou réduite ne traite pas ses messages)
        app.db.send(Cmd::SetVault(Some(dir)));
        // la base ne sert qu'à synchroniser plusieurs appareils
        if !app.cfg.database_url.trim().is_empty() {
            app.connect();
        }
        app
    }

    fn notice(&mut self, msg: impl Into<String>, error: bool, now: f64) {
        self.notices.push((msg.into(), now + if error { 7.0 } else { 3.5 }, error));
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

    fn palette(&self) -> theme::Palette {
        theme::palette(self.cfg.dark_mode)
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

impl eframe::App for MnemoApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.now = now;
        self.watch_css(&ctx, now);
        self.apply_theme(&ctx);
        self.handle_messages(now);
        self.apply_plugin_actions(&ctx, now);
        self.tick_autosave(&ctx, now);
        self.tick_whiteboards(now, false);
        if let Some(t) = self.search_due {
            if now >= t {
                self.search_due = None;
                if !self.search_q.trim().is_empty() {
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

        self.shortcuts(&ctx);
        self.update_plugin_outputs(&ctx, now);

        let p = self.palette();
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
        if let Some(id) = self.focus_editor.take() {
            ctx.memory_mut(|m| m.request_focus(id));
        }

        if self.show_settings {
            self.settings_window(&ctx, now);
        }
        self.conflict_window(&ctx, now);
        self.dialogs(&ctx, now);
        self.icon_picker(&ctx);
        self.image_viewer(&ctx);
        if std::mem::take(&mut self.forget_images) {
            ctx.forget_all_images();
        }
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
            } else if let Some(t) = url.strip_prefix(markdown::BOARD_SCHEME) {
                self.actions.push(UiAction::OpenFile(markdown::decode(t)));
            }
        }
        // fichiers / dossiers déposés depuis le système : dans le dossier survolé de l'explorateur, sinon à la racine
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| p.is_absolute()).collect()
        });
        if !dropped.is_empty() {
            // la cible n'est valable que si le survol vient de se terminer (glisser annulé sinon)
            let target = self.os_drop_target.take().filter(|t| now - t.1 < 0.5).map(|t| t.0);
            let tab = &self.tabs[self.active_tab];
            let on_note = target.is_none() && self.open.is_some() && !tab.dashboard && !tab.graph;
            // déposés sur la note ouverte : les images et autres fichiers y sont intégrés, les .md importés
            let (attach, rest): (Vec<_>, Vec<_>) = dropped
                .into_iter()
                .partition(|p| on_note && p.is_file() && !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")));
            for p in attach {
                if let (Some(name), Ok(bytes)) = (p.file_name().and_then(|n| n.to_str()), std::fs::read(&p)) {
                    self.attach_to_open_note(name, bytes);
                }
            }
            if !rest.is_empty() {
                self.actions.push(UiAction::ImportFiles(rest, target.unwrap_or_default()));
            }
        }
        self.process_actions(&ctx, now);
        self.apply_plugin_actions(&ctx, now);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // whiteboards modifiés (texte en cours de saisie compris) : écrits ici même, avant de quitter
        self.tick_whiteboards(self.now, true);
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
        // Last chance to persist unsaved text (two rounds: text typed while a save was already in flight)
        for _ in 0..2 {
            self.flush_save();
            if !self.open.as_ref().is_some_and(|n| n.saving.is_some()) {
                break;
            }
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let mut rejected = false;
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                match self.rx.recv_timeout(left) {
                    Ok(Msg::Saved(note)) => {
                        if let Some(n) = self.open.as_mut().filter(|n| n.id == note.id) {
                            n.version = note.version;
                            n.saved = n.saving.take().unwrap_or_default();
                        }
                        break;
                    }
                    Ok(Msg::Conflict(_)) | Ok(Msg::Error(_)) => {
                        rejected = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            if rejected {
                // the file changed on disk meanwhile, or could not be written: keep the text in a separate note
                if let Some(n) = &self.open {
                    let path = format!("{} (conflit {})", n.path, chrono::Local::now().format("%Y-%m-%d %H-%M-%S"));
                    self.db.send(Cmd::Create { path, content: n.content.clone(), open: false, if_missing: false });
                    let _ = self.rx.recv_timeout(std::time::Duration::from_secs(2));
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                break;
            }
        }
        // titre / description d'un ticket en cours de saisie
        self.flush_ticket_on_exit();
        self.cfg.save();
    }
}

const WELCOME: &str = include_str!("../assets/texts/welcome.md");
