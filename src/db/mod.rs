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
mod notes;

use board::*;
use connection::*;
use notes::*;

const SCHEMA: &str = include_str!("../schema.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadPurpose {
    Open,
    Refresh,
}

#[derive(Debug)]
pub enum Cmd {
    Connect(String),
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
}

#[derive(Debug)]
pub enum Msg {
    Connecting,
    Connected,
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
    Graph(GraphData),
    Plugins(HashMap<String, PluginState>),
    Board(Board),
    #[allow(dead_code)]
    TicketCreated(Uuid),
    /// (project, message)
    GithubDone(Option<Uuid>, String),
    GithubFailed(Option<Uuid>, String),
    GithubUser(Result<String, String>),
    Event(VaultEvent),
    /// The live connection was re-established: state must be re-fetched.
    Resync,
    Error(String),
}

#[derive(Clone)]
pub struct DbHandle {
    tx: mpsc::UnboundedSender<Cmd>,
}

impl DbHandle {
    pub fn send(&self, cmd: Cmd) {
        let _ = self.tx.send(cmd);
    }
}

/// Spawns the worker thread. `notify` is called after each message so the UI repaints.
pub fn spawn(
    client_id: String,
    out: std::sync::mpsc::Sender<Msg>,
    notify: impl Fn() + Send + Sync + 'static,
) -> DbHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let notify: Arc<dyn Fn() + Send + Sync> = Arc::new(notify);
    std::thread::Builder::new()
        .name("mnemo-db".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(
                Worker { client_id, out, notify, client: None, url: String::new(), listener: None, github: None }
                    .run(rx),
            );
        })
        .expect("spawn db thread");
    DbHandle { tx }
}

struct Worker {
    client_id: String,
    out: std::sync::mpsc::Sender<Msg>,
    notify: Arc<dyn Fn() + Send + Sync>,
    client: Option<Client>,
    url: String,
    listener: Option<tokio::task::JoinHandle<()>>,
    github: Option<GhConfig>,
}

type Res<T> = Result<T, tokio_postgres::Error>;

impl Worker {
    fn emit(&self, msg: Msg) {
        let _ = self.out.send(msg);
        (self.notify)();
    }

    async fn run(mut self, mut rx: mpsc::UnboundedReceiver<Cmd>) {
        while let Some(cmd) = rx.recv().await {
            if let Cmd::Connect(url) = cmd {
                self.connect(url).await;
                continue;
            }
            if let Cmd::SetGithub(cfg) = cmd {
                self.github = cfg;
                continue;
            }
            if let Cmd::GithubTest = cmd {
                let cfg = self.github.clone();
                let r = match cfg {
                    Some(c) => tokio::task::spawn_blocking(move || github::whoami(&c))
                        .await
                        .unwrap_or_else(|e| Err(e.to_string())),
                    None => Err("Aucun token GitHub configuré".into()),
                };
                self.emit(Msg::GithubUser(r));
                continue;
            }
            // Transparent reconnection of the query connection
            if self.client.as_ref().is_none_or(|c| c.is_closed()) {
                if self.url.is_empty() {
                    continue;
                }
                match open_client(&self.url).await {
                    Ok(c) => self.client = Some(c),
                    Err(e) => {
                        self.emit(Msg::Error(format!("Base de données injoignable : {}", pg_err(&e))));
                        continue;
                    }
                }
            }
            if let Err(e) = self.handle(cmd).await {
                self.emit(Msg::Error(pg_err(&e)));
            }
        }
    }

    fn c(&self) -> &Client {
        self.client.as_ref().expect("connected")
    }

    /// Aiguille chaque commande vers le module qui la traite.
    async fn handle(&mut self, cmd: Cmd) -> Res<()> {
        let em = Emitter { out: self.out.clone(), notify: self.notify.clone() };
        match cmd {
            // traitées directement dans `run`
            Cmd::Connect(_) | Cmd::SetGithub(_) | Cmd::GithubTest => unreachable!(),
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
            cmd => self.note_cmd(cmd, &em).await,
        }
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
