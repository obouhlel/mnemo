//! Background database worker.
//!
//! The UI thread sends [`Cmd`]s through a channel; a tokio runtime running on its own thread
//! executes them sequentially on one connection and sends back [`Msg`]s. A second dedicated
//! connection `LISTEN`s on the `vault_events` channel fed by the Postgres triggers; this is
//! what provides live refresh between every running instance.

use crate::board::*;
use crate::github::{self, GhConfig};
use crate::model::*;
use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio_postgres::{AsyncMessage, Client, Connection, NoTls, Row};
use uuid::Uuid;

mod board;
mod connection;
mod github_sync;
mod vault_sync;

use board::*;
use connection::*;
pub use vault_sync::VaultStats;
pub(crate) use vault_sync::{DiskFile, note_file, walk, walk_files};

/// Notes Markdown d'un dossier du disque : (chemin relatif sans `.md`, fichier), sous-dossiers, entrées ignorées.
pub fn walk_markdown(root: &std::path::Path) -> (Vec<(String, std::path::PathBuf)>, Vec<String>, usize) {
    let (files, dirs, skipped) = vault_sync::walk(root);
    (files.into_values().map(|f| (f.rel, f.abs)).collect(), dirs, skipped)
}

const SCHEMA: &str = include_str!("../schema.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPurpose {
    Open,
    Refresh,
}

#[derive(Debug)]
pub enum Cmd {
    Connect(String),
    /// Arrête la synchronisation avec la base (Mnemo continue en local).
    Disconnect,
    /// Stockage local : dossier des notes (voir `crate::local`).
    SetRoot(std::path::PathBuf),
    ListNotes,
    ListFolders,
    Load {
        id: Uuid,
        purpose: LoadPurpose,
    },
    /// Opens the note designated by a wikilink, creating it if needed.
    OpenLink(String),
    Create {
        path: String,
        content: String,
        open: bool,
        if_missing: bool,
    },
    Save {
        id: Uuid,
        content: String,
        base_version: i32,
    },
    Rename {
        id: Uuid,
        new_path: String,
    },
    Delete(Uuid),
    CreateFolder(String),
    RenameFolder {
        from: String,
        to: String,
    },
    DeleteFolder(String),
    Search(String),
    /// Icônes de l'explorateur : chemin (dossier ou note) → icône
    ListIcons,
    /// Pièces jointes du dossier (images, PDF, whiteboards…) : chemins relatifs avec extension
    ListFiles,
    /// Enregistre une pièce jointe (image collée ou déposée) ; le nom est rendu unique.
    /// `insert` : l'interface insère ensuite `![[nom]]` dans la note ouverte.
    SaveFile {
        path: String,
        bytes: Vec<u8>,
        insert: bool,
    },
    DeleteFile(String),
    /// Réécrit une pièce jointe (whiteboard enregistré)
    WriteFile {
        path: String,
        bytes: Vec<u8>,
    },
    SetIcon {
        path: String,
        icon: Option<String>,
    },
    Backlinks(Uuid),
    Tags,
    Graph,
    LoadPlugins,
    SetPlugin {
        id: String,
        state: PluginState,
    },
    // ---- dashboard
    SetGithub(Option<GhConfig>),
    GithubTest,
    /// Dépôts visibles avec le token (choix du dépôt d'un projet)
    GithubRepos,
    LoadBoard,
    CreateProject {
        name: String,
        description: String,
        repo: Option<String>,
    },
    UpdateProject {
        id: Uuid,
        name: String,
        description: String,
        repo: Option<String>,
    },
    DeleteProject(Uuid),
    CreateTicket {
        project_id: Uuid,
        title: String,
        tags: Vec<String>,
        status: Status,
    },
    UpdateTicket {
        id: Uuid,
        patch: TicketPatch,
    },
    DeleteTicket(Uuid),
    LinkReport {
        ticket: Uuid,
        note: Uuid,
    },
    UnlinkReport {
        ticket: Uuid,
        note: Uuid,
    },
    /// Creates a note, links it to the ticket and opens it
    CreateReport {
        ticket: Uuid,
        path: String,
        content: String,
    },
    GithubSync(Uuid),
    GithubPublish(Uuid),
    // ---- dossier du coffre sur le disque (voir vault_sync.rs)
    /// Lie (ou délie avec `None`) un dossier de coffre ; la synchronisation repart de zéro.
    SetVault(Option<std::path::PathBuf>),
    /// Compare le dossier lié à la base et recopie les changements dans les deux sens.
    VaultScan,
}

#[derive(Debug)]
pub enum Msg {
    Connecting,
    Connected,
    /// Synchronisation arrêtée à la demande (Mnemo reste en local).
    Disconnected,
    /// Le dossier des notes est indexé.
    LocalReady,
    /// Changement dans le dossier des notes (par Mnemo, un autre éditeur ou la synchronisation).
    LocalEvent(VaultEvent),
    /// Une note a adopté l'identifiant qu'elle a dans la base (première synchronisation).
    IdChanged {
        old: Uuid,
        new: Uuid,
    },
    ConnectionFailed(String),
    Live(bool),
    Notes(Vec<NoteMeta>),
    Folders(Vec<String>),
    Loaded {
        note: Note,
        purpose: LoadPurpose,
    },
    Created {
        note: Note,
        open: bool,
    },
    Saved(Note),
    Conflict(Note),
    Renamed {
        note: Note,
        relinked: usize,
    },
    Deleted(Uuid),
    SearchResults {
        query: String,
        hits: Vec<SearchHit>,
    },
    Backlinks {
        id: Uuid,
        links: Vec<Backlink>,
    },
    Tags(Vec<(String, i64)>),
    /// chemin en minuscules → icône
    Icons(HashMap<String, String>),
    Files(Vec<String>),
    FileSaved {
        path: String,
        insert: bool,
    },
    Graph(GraphData),
    Plugins(HashMap<String, PluginState>),
    Board(Board),
    #[allow(dead_code)]
    TicketCreated(Uuid),
    /// (project, message)
    GithubDone(Option<Uuid>, String),
    GithubFailed(Option<Uuid>, String),
    GithubUser(Result<String, String>),
    GithubRepos(Result<Vec<String>, String>),
    Event(VaultEvent),
    /// Passage de synchronisation du dossier du coffre (envoyé au premier passage et quand il y a eu des changements).
    VaultSynced {
        stats: VaultStats,
        first: bool,
    },
    /// The live connection was re-established: state must be re-fetched.
    Resync,
    Error(String),
}

/// Envoie chaque commande au bon thread : notes, dossiers, recherche, icônes et plugins au stockage local
/// (`crate::local`), tableau de bord, GitHub, connexion et synchronisation au worker PostgreSQL.
#[derive(Clone)]
pub struct DbHandle {
    tx: mpsc::UnboundedSender<Cmd>,
    local: std::sync::mpsc::Sender<Cmd>,
}

impl Cmd {
    fn is_local(&self) -> bool {
        matches!(
            self,
            Cmd::SetRoot(_)
                | Cmd::ListNotes
                | Cmd::ListFolders
                | Cmd::Load { .. }
                | Cmd::OpenLink(_)
                | Cmd::Create { .. }
                | Cmd::Save { .. }
                | Cmd::Rename { .. }
                | Cmd::Delete(_)
                | Cmd::CreateFolder(_)
                | Cmd::RenameFolder { .. }
                | Cmd::DeleteFolder(_)
                | Cmd::Search(_)
                | Cmd::Backlinks(_)
                | Cmd::Tags
                | Cmd::Graph
                | Cmd::LoadPlugins
                | Cmd::SetPlugin { .. }
                | Cmd::ListIcons
                | Cmd::SetIcon { .. }
                | Cmd::ListFiles
                | Cmd::SaveFile { .. }
                | Cmd::DeleteFile(_)
                | Cmd::WriteFile { .. }
        )
    }
}

impl DbHandle {
    pub fn send(&self, cmd: Cmd) {
        if cmd.is_local() {
            let _ = self.local.send(cmd);
        } else {
            let _ = self.tx.send(cmd);
        }
    }
}

/// Spawns the worker thread. `notify` is called after each message so the UI repaints.
pub fn spawn(
    client_id: String,
    out: std::sync::mpsc::Sender<Msg>,
    notify: impl Fn() + Send + Sync + 'static,
    shared: crate::local::shared::SharedState,
) -> DbHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(notify);
    let (local_tx, local_rx) = std::sync::mpsc::channel();
    crate::local::spawn(client_id.clone(), out.clone(), notify.clone(), shared.clone(), local_rx);
    std::thread::Builder::new()
        .name("mnemo-db".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(
                Worker {
                    client_id,
                    out,
                    notify,
                    client: None,
                    url: String::new(),
                    listener: None,
                    github: None,
                    vault: None,
                    shared,
                    want_url: String::new(),
                    last_try: None,
                }
                .run(rx),
            );
        })
        .expect("spawn db thread");
    DbHandle { tx, local: local_tx }
}

struct Worker {
    client_id: String,
    out: std::sync::mpsc::Sender<Msg>,
    notify: Arc<dyn Fn() + Send + Sync>,
    client: Option<Client>,
    url: String,
    listener: Option<tokio::task::JoinHandle<()>>,
    github: Option<GhConfig>,
    vault: Option<vault_sync::Vault>,
    shared: crate::local::shared::SharedState,
    /// base demandée par l'utilisateur (vide : local uniquement) ; le worker s'y reconnecte tout seul
    want_url: String,
    last_try: Option<std::time::Instant>,
}

type Res<T> = Result<T, tokio_postgres::Error>;

impl Worker {
    fn emit(&self, msg: Msg) {
        let _ = self.out.send(msg);
        (self.notify)();
    }

    /// Boucle du worker : les commandes de l'interface, et toutes les 2 s, sans dépendre de l'affichage
    /// (fenêtre réduite ou cachée), la synchronisation dossier ↔ base et la reconnexion si elle est coupée.
    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Cmd>) {
        let mut tick = tokio::time::interval(Duration::from_secs(2));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                cmd = rx.recv() => match cmd {
                    Some(cmd) => self.dispatch(cmd).await,
                    None => break,
                },
                _ = tick.tick() => self.tick().await,
            }
        }
    }

    /// Travail périodique : reconnexion (toutes les 30 s) puis comparaison dossier ↔ base.
    async fn tick(&mut self) {
        if self.want_url.is_empty() {
            return;
        }
        if self.client.as_ref().is_none_or(|c| c.is_closed()) {
            if self.last_try.is_some_and(|t| t.elapsed() < Duration::from_secs(30)) {
                return;
            }
            let url = self.want_url.clone();
            self.connect(url).await;
            return;
        }
        if self.vault.is_some() {
            self.scan_or_go_offline().await;
        }
    }

    /// Passage de synchronisation ; une erreur de connexion fait passer en hors ligne (sans alerte répétée).
    async fn scan_or_go_offline(&mut self) {
        let em = Emitter { out: self.out.clone(), notify: self.notify.clone() };
        if let Err(e) = self.vault_scan(&em).await {
            if self.client.as_ref().is_none_or(|c| c.is_closed()) {
                self.client = None;
                self.last_try = Some(std::time::Instant::now());
                if let Some(v) = &mut self.vault {
                    v.reset(); // au retour, tout est recomparé
                }
                self.emit(Msg::ConnectionFailed(pg_err(&e)));
            } else {
                self.emit(Msg::Error(pg_err(&e)));
            }
        }
    }

    async fn dispatch(&mut self, cmd: Cmd) {
        if let Cmd::Connect(url) = cmd {
            self.want_url = url.clone();
            self.connect(url).await;
            return;
        }
        if let Cmd::Disconnect = cmd {
            if let Some(l) = self.listener.take() {
                l.abort();
            }
            self.client = None;
            self.url.clear();
            self.want_url.clear();
            if let Some(v) = &mut self.vault {
                v.reset(); // à la reconnexion, tout est recomparé
            }
            self.emit(Msg::Disconnected);
            return;
        }
        if let Cmd::SetVault(dir) = cmd {
            self.vault = dir.map(vault_sync::Vault::new);
            return;
        }
        if let Cmd::VaultScan = cmd {
            // demandé par l'interface (changement venu d'un autre appareil) : seulement si la base répond
            if self.vault.is_some() && self.client.as_ref().is_some_and(|c| !c.is_closed()) {
                self.scan_or_go_offline().await;
            }
            return;
        }
        if let Cmd::SetGithub(cfg) = cmd {
            self.github = cfg;
            return;
        }
        if let Cmd::GithubTest = cmd {
            let cfg = self.github.clone();
            let r = match cfg {
                Some(c) => {
                    tokio::task::spawn_blocking(move || github::whoami(&c)).await.unwrap_or_else(|e| Err(e.to_string()))
                }
                None => Err("Aucun token GitHub configuré".into()),
            };
            self.emit(Msg::GithubUser(r));
            return;
        }
        if let Cmd::GithubRepos = cmd {
            let r = match self.github.clone() {
                Some(c) => tokio::task::spawn_blocking(move || github::list_repos(&c))
                    .await
                    .unwrap_or_else(|e| Err(e.to_string())),
                None => Err("Aucun token GitHub configuré".into()),
            };
            self.emit(Msg::GithubRepos(r));
            return;
        }
        // Transparent reconnection of the query connection
        if self.client.as_ref().is_none_or(|c| c.is_closed()) {
            if self.url.is_empty() {
                return;
            }
            match open_client(&self.url).await {
                Ok(c) => self.client = Some(c),
                Err(e) => {
                    // hors ligne : l'interface le signale une seule fois, le worker réessaie toutes les 30 s
                    self.client = None;
                    self.last_try = Some(std::time::Instant::now());
                    self.emit(Msg::ConnectionFailed(pg_err(&e)));
                    return;
                }
            }
        }
        if let Err(e) = self.handle(cmd).await {
            self.emit(Msg::Error(pg_err(&e)));
        }
    }

    /// Connexion de requêtes (garantie ouverte par `run` avant chaque commande).
    fn conn(&self) -> &Client {
        self.client.as_ref().expect("connected")
    }

    /// Aiguille chaque commande vers le module qui la traite.
    async fn handle(&mut self, cmd: Cmd) -> Res<()> {
        let em = Emitter { out: self.out.clone(), notify: self.notify.clone() };
        match cmd {
            // traitées directement dans `run`
            Cmd::Connect(_)
            | Cmd::Disconnect
            | Cmd::SetGithub(_)
            | Cmd::GithubTest
            | Cmd::GithubRepos
            | Cmd::SetVault(_) => {
                unreachable!()
            }
            Cmd::VaultScan => unreachable!(),
            Cmd::LoadBoard
            | Cmd::CreateProject { .. }
            | Cmd::UpdateProject { .. }
            | Cmd::DeleteProject(_)
            | Cmd::CreateTicket { .. }
            | Cmd::UpdateTicket { .. }
            | Cmd::DeleteTicket(_)
            | Cmd::LinkReport { .. }
            | Cmd::UnlinkReport { .. }
            | Cmd::CreateReport { .. } => self.board_cmd(cmd, &em).await,
            Cmd::GithubPublish(_) | Cmd::GithubSync(_) => self.github_cmd(cmd, &em).await,
            // notes, dossiers, recherche… : traités par le stockage local (`Cmd::is_local`), jamais envoyés ici
            _ => Ok(()),
        }
    }
}

pub(super) const NOTE_COLS: &str = "id, path, content, version, updated_by, updated_at";

pub(super) fn note_from(r: &Row) -> Note {
    Note {
        id: r.get(0),
        path: r.get(1),
        content: r.get(2),
        version: r.get(3),
        updated_by: r.get(4),
        updated_at: r.get(5),
    }
}

pub(super) struct Emitter {
    pub(super) out: std::sync::mpsc::Sender<Msg>,
    pub(super) notify: Arc<dyn Fn() + Send + Sync>,
}
impl Emitter {
    pub(super) fn emit(&self, msg: Msg) {
        let _ = self.out.send(msg);
        (self.notify)();
    }
}

/// Opens a transaction tagged with this instance's client id
/// (read by the triggers to fill `updated_by`, so each instance can ignore its own echoes).
pub(super) async fn tagged_tx<'a>(client: &'a mut Client, client_id: &str) -> Res<tokio_postgres::Transaction<'a>> {
    let t = client.transaction().await?;
    t.execute("SELECT set_config('app.client_id', $1, true)", &[&client_id]).await?;
    Ok(t)
}

pub(super) fn is_unique_violation(e: &tokio_postgres::Error) -> bool {
    e.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
}

pub(super) fn pg_err(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => db.message().to_string(),
        None => {
            let mut s = e.to_string();
            let mut src = std::error::Error::source(e);
            while let Some(inner) = src {
                s.push_str(&format!(" : {inner}"));
                src = inner.source();
            }
            s
        }
    }
}
