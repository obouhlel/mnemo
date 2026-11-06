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

const SCHEMA: &str = include_str!("schema.sql");

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
    Load { id: Uuid, purpose: LoadPurpose },
    /// Opens the note designated by a wikilink, creating it if needed.
    OpenLink(String),
    Create { path: String, content: String, open: bool, if_missing: bool },
    Save { id: Uuid, content: String, base_version: i32 },
    Rename { id: Uuid, new_path: String },
    Delete(Uuid),
    CreateFolder(String),
    RenameFolder { from: String, to: String },
    DeleteFolder(String),
    Search(String),
    Backlinks(Uuid),
    Tags,
    Graph,
    LoadPlugins,
    SetPlugin { id: String, state: PluginState },
    // ---- dashboard
    SetGithub(Option<GhConfig>),
    GithubTest,
    LoadBoard,
    CreateProject { name: String, description: String, repo: Option<String> },
    UpdateProject { id: Uuid, name: String, description: String, repo: Option<String> },
    DeleteProject(Uuid),
    CreateTicket { project_id: Uuid, title: String, tags: Vec<String>, status: Status },
    UpdateTicket { id: Uuid, patch: TicketPatch },
    DeleteTicket(Uuid),
    LinkReport { ticket: Uuid, note: Uuid },
    UnlinkReport { ticket: Uuid, note: Uuid },
    /// Creates a note, links it to the ticket and opens it
    CreateReport { ticket: Uuid, path: String, content: String },
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
    Loaded { note: Note, purpose: LoadPurpose },
    Created { note: Note, open: bool },
    Saved(Note),
    Conflict(Note),
    Renamed { note: Note, relinked: usize },
    Deleted(Uuid),
    SearchResults { query: String, hits: Vec<SearchHit> },
    Backlinks { id: Uuid, links: Vec<Backlink> },
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
pub fn spawn(client_id: String, out: std::sync::mpsc::Sender<Msg>, notify: impl Fn() + Send + Sync + 'static) -> DbHandle {
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
            rt.block_on(Worker { client_id, out, notify, client: None, url: String::new(), listener: None, github: None }.run(rx));
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
                    Some(c) => tokio::task::spawn_blocking(move || github::whoami(&c)).await.unwrap_or_else(|e| Err(e.to_string())),
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

    async fn connect(&mut self, url: String) {
        self.emit(Msg::Connecting);
        if let Some(l) = self.listener.take() {
            l.abort();
        }
        self.client = None;
        let client = match open_client(&url).await {
            Ok(c) => c,
            Err(e) => return self.emit(Msg::ConnectionFailed(pg_err(&e))),
        };
        // Migration, serialized across instances with an advisory lock
        let migrate = async {
            client.execute("SELECT pg_advisory_lock(424242)", &[]).await?;
            let r = client.batch_execute(SCHEMA).await;
            client.execute("SELECT pg_advisory_unlock(424242)", &[]).await?;
            r
        };
        if let Err(e) = migrate.await {
            return self.emit(Msg::ConnectionFailed(format!("Migration du schéma impossible : {}", pg_err(&e))));
        }
        self.client = Some(client);
        self.url = url.clone();
        self.listener = Some(tokio::spawn(listen_loop(url, self.out.clone(), self.notify.clone())));
        self.emit(Msg::Connected);
    }

    fn c(&self) -> &Client {
        self.client.as_ref().expect("connected")
    }

    async fn handle(&mut self, cmd: Cmd) -> Res<()> {
        let em = Emitter { out: self.out.clone(), notify: self.notify.clone() };
        match cmd {
            Cmd::Connect(_) => unreachable!(),
            Cmd::ListNotes => {
                let rows = self.c().query("SELECT id, path, version, updated_at FROM notes ORDER BY lower(path)", &[]).await?;
                let notes = rows
                    .iter()
                    .map(|r| NoteMeta { id: r.get(0), path: r.get(1), version: r.get(2), updated_at: r.get(3) })
                    .collect();
                em.emit(Msg::Notes(notes));
            }
            Cmd::ListFolders => {
                let rows = self.c().query("SELECT path FROM folders ORDER BY lower(path)", &[]).await?;
                em.emit(Msg::Folders(rows.iter().map(|r| r.get(0)).collect()));
            }
            Cmd::Load { id, purpose } => {
                match self.c().query_opt(&format!("SELECT {NOTE_COLS} FROM notes WHERE id = $1"), &[&id]).await? {
                    Some(r) => em.emit(Msg::Loaded { note: note_from(&r), purpose }),
                    None if purpose == LoadPurpose::Open => em.emit(Msg::Error("Note introuvable".into())),
                    None => {}
                }
            }
            Cmd::OpenLink(link) => {
                let link = link.trim().trim_end_matches(".md").to_string();
                if let Some(r) = self.c().query_opt(RESOLVE_SQL, &[&link]).await? {
                    em.emit(Msg::Loaded { note: note_from(&r), purpose: LoadPurpose::Open });
                } else {
                    match normalize_path(&link) {
                        Ok(path) => {
                            let content = String::new();
                            Box::pin(self.handle(Cmd::Create { path, content, open: true, if_missing: true })).await?;
                        }
                        Err(e) => em.emit(Msg::Error(e)),
                    }
                }
            }
            Cmd::Create { path, content, open, if_missing } => {
                if if_missing
                    && let Some(r) = self.c().query_opt(&format!("SELECT {NOTE_COLS} FROM notes WHERE lower(path) = lower($1)"), &[&path]).await?
                {
                    if open {
                        em.emit(Msg::Loaded { note: note_from(&r), purpose: LoadPurpose::Open });
                    }
                    return Ok(());
                }
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                let r = t.query_one(&format!("INSERT INTO notes (path, content) VALUES ($1, $2) RETURNING {NOTE_COLS}"), &[&path, &content]).await;
                match r {
                    Ok(row) => {
                        t.commit().await?;
                        em.emit(Msg::Created { note: note_from(&row), open });
                    }
                    Err(e) if is_unique_violation(&e) => em.emit(Msg::Error(format!("« {path} » existe déjà"))),
                    Err(e) => return Err(e),
                }
            }
            Cmd::Save { id, content, base_version } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                let cur = t.query_opt(&format!("SELECT {NOTE_COLS} FROM notes WHERE id = $1 FOR UPDATE"), &[&id]).await?;
                let Some(cur) = cur else {
                    em.emit(Msg::Error("La note a été supprimée entre-temps".into()));
                    return Ok(());
                };
                let current = note_from(&cur);
                if current.version != base_version {
                    t.rollback().await?;
                    em.emit(Msg::Conflict(current));
                    return Ok(());
                }
                let row = t.query_opt(&format!("UPDATE notes SET content = $2 WHERE id = $1 RETURNING {NOTE_COLS}"), &[&id, &content]).await?;
                t.commit().await?;
                em.emit(Msg::Saved(row.map(|r| note_from(&r)).unwrap_or(current)));
            }
            Cmd::Rename { id, new_path } => {
                let mut t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                let old: Option<String> = t.query_opt("SELECT path FROM notes WHERE id = $1", &[&id]).await?.map(|r| r.get(0));
                let Some(old) = old else { return Ok(()) };
                if old == new_path {
                    return Ok(()); // nothing to do (e.g. the same rename sent twice)
                }
                let row = match t.query_one(&format!("UPDATE notes SET path = $2 WHERE id = $1 RETURNING {NOTE_COLS}"), &[&id, &new_path]).await {
                    Ok(r) => r,
                    Err(e) if is_unique_violation(&e) => {
                        em.emit(Msg::Error(format!("« {new_path} » existe déjà")));
                        return Ok(());
                    }
                    Err(e) => return Err(e),
                };
                let relinked = relink(&mut t, id, &old, &new_path).await?;
                t.commit().await?;
                em.emit(Msg::Renamed { note: note_from(&row), relinked });
            }
            Cmd::Delete(id) => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute("DELETE FROM notes WHERE id = $1", &[&id]).await?;
                t.commit().await?;
                em.emit(Msg::Deleted(id));
            }
            Cmd::CreateFolder(path) => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute("INSERT INTO folders (path) VALUES ($1) ON CONFLICT DO NOTHING", &[&path]).await?;
                t.commit().await?;
            }
            Cmd::RenameFolder { from, to } => {
                if to.to_lowercase().starts_with(&(from.to_lowercase() + "/")) {
                    em.emit(Msg::Error("Impossible de déplacer un dossier dans lui-même".into()));
                    return Ok(());
                }
                let mut t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                let notes = t.query(&format!("SELECT id, path FROM notes WHERE {PREFIXED}"), &[&from]).await?;
                for n in &notes {
                    let (id, path): (Uuid, String) = (n.get(0), n.get(1));
                    let np = format!("{to}{}", &path[from.len()..]);
                    if let Err(e) = t.execute("UPDATE notes SET path = $2 WHERE id = $1", &[&id, &np]).await {
                        if is_unique_violation(&e) {
                            em.emit(Msg::Error(format!("« {np} » existe déjà : renommage annulé")));
                            return Ok(());
                        }
                        return Err(e);
                    }
                    relink(&mut t, id, &path, &np).await?;
                }
                let folders = t.query(&format!("DELETE FROM folders WHERE {PREFIXED} RETURNING path"), &[&from]).await?;
                for f in &folders {
                    let p: String = f.get(0);
                    t.execute("INSERT INTO folders (path) VALUES ($1) ON CONFLICT DO NOTHING", &[&format!("{to}{}", &p[from.len()..])]).await?;
                }
                t.commit().await?;
            }
            Cmd::DeleteFolder(path) => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute(&format!("DELETE FROM notes WHERE {PREFIXED}"), &[&path]).await?;
                t.execute(&format!("DELETE FROM folders WHERE {PREFIXED}"), &[&path]).await?;
                t.commit().await?;
            }
            Cmd::Search(q) => {
                let words: Vec<String> = q
                    .split_whitespace()
                    .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect::<String>())
                    .filter(|w| !w.is_empty())
                    .collect();
                let tsq = words.iter().map(|w| format!("{w}:*")).collect::<Vec<_>>().join(" & ");
                let like = format!("%{}%", q.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
                let rows = self
                    .c()
                    .query(
                        "SELECT id, path,
                           CASE WHEN $1 <> '' THEN ts_headline('simple', content, to_tsquery('simple', $1),
                             'StartSel=\u{1}, StopSel=\u{2}, MaxFragments=2, MaxWords=18, MinWords=6, FragmentDelimiter=\" … \"')
                           ELSE left(content, 120) END
                         FROM notes
                         WHERE ($1 <> '' AND search @@ to_tsquery('simple', $1)) OR path ILIKE $2
                         ORDER BY (CASE WHEN $1 <> '' THEN ts_rank(search, to_tsquery('simple', $1)) ELSE 0 END)
                                  + (CASE WHEN path ILIKE $2 THEN 1 ELSE 0 END) DESC, path
                         LIMIT 50",
                        &[&tsq, &like],
                    )
                    .await?;
                let hits = rows.iter().map(|r| SearchHit { id: r.get(0), path: r.get(1), snippet: r.get(2) }).collect();
                em.emit(Msg::SearchResults { query: q, hits });
            }
            Cmd::Backlinks(id) => {
                let Some(row) = self.c().query_opt("SELECT path FROM notes WHERE id = $1", &[&id]).await? else { return Ok(()) };
                let path: String = row.get(0);
                let base = basename(&path).to_string();
                let rows = self
                    .c()
                    .query(
                        "SELECT DISTINCT n.id, n.path, n.content FROM links l JOIN notes n ON n.id = l.source_id
                         WHERE (lower(l.target) = lower($1) OR lower(l.target) = lower($2)) AND n.id <> $3
                         ORDER BY n.path",
                        &[&path, &base, &id],
                    )
                    .await?;
                let (pl, bl) = (path.to_lowercase(), base.to_lowercase());
                let links = rows
                    .iter()
                    .map(|r| {
                        let content: String = r.get(2);
                        let snippets = content
                            .lines()
                            .filter(|line| {
                                let l = line.to_lowercase();
                                [&pl, &bl].iter().any(|t| ["]]", "|", "#"].iter().any(|end| l.contains(&format!("[[{t}{end}"))))
                            })
                            .take(3)
                            .map(|l| l.trim().chars().take(200).collect())
                            .collect();
                        Backlink { id: r.get(0), path: r.get(1), snippets }
                    })
                    .collect();
                em.emit(Msg::Backlinks { id, links });
            }
            Cmd::Tags => {
                let rows = self
                    .c()
                    .query(
                        r"SELECT lower(m[1]), count(DISTINCT id)
                          FROM notes, regexp_matches(content, '(?:^|\s)#([[:alpha:]_][[:alnum:]_/-]*)', 'g') AS m
                          GROUP BY 1 ORDER BY 2 DESC, 1",
                        &[],
                    )
                    .await?;
                em.emit(Msg::Tags(rows.iter().map(|r| (r.get(0), r.get(1))).collect()));
            }
            Cmd::Graph => {
                let nodes = self.c().query("SELECT id::text, path FROM notes", &[]).await?;
                let edges = self
                    .c()
                    .query(
                        "SELECT l.source_id::text, t.id::text, l.target FROM links l
                         LEFT JOIN LATERAL (
                           SELECT n.id FROM notes n
                           WHERE lower(n.path) = lower(l.target) OR lower(regexp_replace(n.path, '^.*/', '')) = lower(l.target)
                           ORDER BY (lower(n.path) = lower(l.target)) DESC, length(n.path) LIMIT 1
                         ) t ON true",
                        &[],
                    )
                    .await?;
                let mut g = GraphData { nodes: nodes.iter().map(|r| (r.get(0), r.get(1), false)).collect(), edges: vec![] };
                let mut ghosts = std::collections::HashSet::new();
                for e in &edges {
                    let src: String = e.get(0);
                    let tgt: Option<String> = e.get(1);
                    match tgt {
                        Some(t) if t != src => g.edges.push((src, t)),
                        Some(_) => {}
                        None => {
                            let name: String = e.get(2);
                            let key = format!("ghost:{}", name.to_lowercase());
                            if ghosts.insert(key.clone()) {
                                g.nodes.push((key.clone(), name, true));
                            }
                            g.edges.push((src, key));
                        }
                    }
                }
                em.emit(Msg::Graph(g));
            }
            Cmd::LoadPlugins => {
                let rows = self.c().query("SELECT id, enabled, settings FROM plugin_state", &[]).await?;
                let map = rows
                    .iter()
                    .map(|r| {
                        let settings: serde_json::Value = r.get(2);
                        let settings = settings.as_object().cloned().unwrap_or_default();
                        (r.get::<_, String>(0), PluginState { enabled: r.get(1), settings })
                    })
                    .collect();
                em.emit(Msg::Plugins(map));
            }
            Cmd::SetPlugin { id, state } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                let settings = serde_json::Value::Object(state.settings);
                t.execute(
                    "INSERT INTO plugin_state (id, enabled, settings) VALUES ($1, $2, $3)
                     ON CONFLICT (id) DO UPDATE SET enabled = EXCLUDED.enabled, settings = EXCLUDED.settings",
                    &[&id, &state.enabled, &settings],
                )
                .await?;
                t.commit().await?;
            }
            Cmd::SetGithub(_) | Cmd::GithubTest => unreachable!(),
            Cmd::LoadBoard => {
                let board = load_board(self.c()).await?;
                em.emit(Msg::Board(board));
            }
            Cmd::CreateProject { name, description, repo } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                match t
                    .execute(
                        "INSERT INTO projects (name, description, github_repo, position)
                         VALUES ($1, $2, $3, COALESCE((SELECT max(position) FROM projects), 0) + 1)",
                        &[&name.trim(), &description, &repo],
                    )
                    .await
                {
                    Ok(_) => t.commit().await?,
                    Err(e) if is_unique_violation(&e) => em.emit(Msg::Error(format!("Le projet « {name} » existe déjà"))),
                    Err(e) => return Err(e),
                }
            }
            Cmd::UpdateProject { id, name, description, repo } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                match t
                    .execute("UPDATE projects SET name = $2, description = $3, github_repo = $4 WHERE id = $1", &[&id, &name.trim(), &description, &repo])
                    .await
                {
                    Ok(_) => t.commit().await?,
                    Err(e) if is_unique_violation(&e) => em.emit(Msg::Error(format!("Le projet « {name} » existe déjà"))),
                    Err(e) => return Err(e),
                }
            }
            Cmd::DeleteProject(id) => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute("DELETE FROM projects WHERE id = $1", &[&id]).await?;
                t.commit().await?;
            }
            Cmd::CreateTicket { project_id, title, tags, status } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                let row = t
                    .query_one(
                        "INSERT INTO tickets (project_id, title, tags, status, position)
                         VALUES ($1, $2, $3, $4, COALESCE((SELECT max(position) FROM tickets WHERE project_id = $1), 0) + 1)
                         RETURNING id",
                        &[&project_id, &title.trim(), &tags, &status.as_str()],
                    )
                    .await?;
                t.commit().await?;
                em.emit(Msg::TicketCreated(row.get(0)));
            }
            Cmd::UpdateTicket { id, patch } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute(
                    "UPDATE tickets SET title = COALESCE($2, title), body = COALESCE($3, body), status = COALESCE($4, status),
                       tags = COALESCE($5, tags), priority = COALESCE($6, priority), project_id = COALESCE($7, project_id)
                     WHERE id = $1",
                    &[&id, &patch.title, &patch.body, &patch.status.map(|s| s.as_str()), &patch.tags, &patch.priority, &patch.project_id],
                )
                .await?;
                t.commit().await?;
                // keep the GitHub issue in step (title, body, labels, open/closed)
                let touches_github = patch.title.is_some() || patch.body.is_some() || patch.status.is_some() || patch.tags.is_some();
                if touches_github && self.github.is_some() {
                    let linked = self.c().query_opt(
                        "SELECT t.github_number FROM tickets t JOIN projects p ON p.id = t.project_id
                         WHERE t.id = $1 AND t.github_number IS NOT NULL AND p.github_repo IS NOT NULL", &[&id]).await?;
                    if linked.is_some()
                        && let Err(e) = self.push_ticket(id).await?
                    {
                        em.emit(Msg::GithubFailed(None, e));
                    }
                }
            }
            Cmd::DeleteTicket(id) => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute("DELETE FROM tickets WHERE id = $1", &[&id]).await?;
                t.commit().await?;
            }
            Cmd::LinkReport { ticket, note } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute("INSERT INTO ticket_reports (ticket_id, note_id) VALUES ($1, $2) ON CONFLICT DO NOTHING", &[&ticket, &note]).await?;
                t.commit().await?;
            }
            Cmd::UnlinkReport { ticket, note } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute("DELETE FROM ticket_reports WHERE ticket_id = $1 AND note_id = $2", &[&ticket, &note]).await?;
                t.commit().await?;
            }
            Cmd::CreateReport { ticket, path, content } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                // unique path: add " (2)", " (3)"… if needed
                let mut candidate = path.clone();
                let mut n = 2;
                while t.query_opt("SELECT 1 FROM notes WHERE lower(path) = lower($1)", &[&candidate]).await?.is_some() {
                    candidate = format!("{path} ({n})");
                    n += 1;
                }
                let row = t.query_one(&format!("INSERT INTO notes (path, content) VALUES ($1, $2) RETURNING {NOTE_COLS}"), &[&candidate, &content]).await?;
                let note = note_from(&row);
                t.execute("INSERT INTO ticket_reports (ticket_id, note_id) VALUES ($1, $2)", &[&ticket, &note.id]).await?;
                t.commit().await?;
                em.emit(Msg::Created { note, open: true });
            }
            Cmd::GithubPublish(id) => match self.github.is_some() {
                false => em.emit(Msg::GithubFailed(None, "Configurez d'abord un token GitHub (Paramètres › GitHub)".into())),
                true => match self.publish_ticket(id).await? {
                    Ok(n) => em.emit(Msg::GithubDone(None, format!("Issue #{n} créée sur GitHub"))),
                    Err(e) => em.emit(Msg::GithubFailed(None, e)),
                },
            },
            Cmd::GithubSync(project) => match self.github.is_some() {
                false => em.emit(Msg::GithubFailed(Some(project), "Configurez d'abord un token GitHub (Paramètres › GitHub)".into())),
                true => match self.sync_project(project).await? {
                    Ok(summary) => em.emit(Msg::GithubDone(Some(project), summary)),
                    Err(e) => em.emit(Msg::GithubFailed(Some(project), e)),
                },
            },
        }
        Ok(())
    }

    // ---------------------------------------------------------------- GitHub helpers
    // The outer Result is a database error, the inner one a GitHub error (shown to the user).

    async fn ticket_with_repo(&self, id: Uuid) -> Res<Option<(Ticket, String)>> {
        let row = self
            .c()
            .query_opt(&format!("SELECT {TICKET_COLS}, p.github_repo FROM tickets t JOIN projects p ON p.id = t.project_id WHERE t.id = $1"), &[&id])
            .await?;
        Ok(row.and_then(|r| r.get::<_, Option<String>>(TICKET_COLS_N).map(|repo| (ticket_from(&r), repo))))
    }

    async fn mark_synced(&mut self, id: Uuid, issue: &RemoteIssue) -> Res<()> {
        let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
        t.execute(
            "UPDATE tickets SET github_number = $2, github_url = $3, github_state = $4, github_synced_at = now() WHERE id = $1",
            &[&id, &issue.number, &issue.url, &if issue.open { "open" } else { "closed" }],
        )
        .await?;
        t.commit().await
    }

    async fn push_ticket(&mut self, id: Uuid) -> Res<Result<(), String>> {
        let Some((tk, repo)) = self.ticket_with_repo(id).await? else { return Ok(Err("Projet sans dépôt GitHub".into())) };
        let Some(number) = tk.github_number else { return Ok(Err("Ticket non publié sur GitHub".into())) };
        let cfg = self.github.clone().expect("github");
        let labels: Vec<String> = tk.tags.iter().map(|t| tag_to_label(t)).collect();
        let r = tokio::task::spawn_blocking(move || github::update_issue(&cfg, &repo, number, &tk.title, &tk.body, tk.status != Status::Done, &labels))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        match r {
            Ok(issue) => {
                self.mark_synced(id, &issue).await?;
                Ok(Ok(()))
            }
            Err(e) => Ok(Err(e)),
        }
    }

    async fn publish_ticket(&mut self, id: Uuid) -> Res<Result<i32, String>> {
        let Some((tk, repo)) = self.ticket_with_repo(id).await? else {
            return Ok(Err("Associez d'abord un dépôt GitHub au projet".into()));
        };
        if let Some(n) = tk.github_number {
            return Ok(Err(format!("Déjà publié (issue #{n})")));
        }
        let cfg = self.github.clone().expect("github");
        let labels: Vec<String> = tk.tags.iter().map(|t| tag_to_label(t)).collect();
        let done = tk.status == Status::Done;
        let r = tokio::task::spawn_blocking(move || {
            let issue = github::create_issue(&cfg, &repo, &tk.title, &tk.body, &labels)?;
            if done { github::update_issue(&cfg, &repo, issue.number, &tk.title, &tk.body, false, &labels) } else { Ok(issue) }
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        match r {
            Ok(issue) => {
                self.mark_synced(id, &issue).await?;
                Ok(Ok(issue.number))
            }
            Err(e) => Ok(Err(e)),
        }
    }

    async fn sync_project(&mut self, project: Uuid) -> Res<Result<String, String>> {
        let Some(row) = self.c().query_opt("SELECT name, github_repo FROM projects WHERE id = $1", &[&project]).await? else {
            return Ok(Err("Projet introuvable".into()));
        };
        let (name, repo): (String, Option<String>) = (row.get(0), row.get(1));
        let Some(repo) = repo else { return Ok(Err(format!("Le projet « {name} » n'a pas de dépôt GitHub"))) };
        let cfg = self.github.clone().expect("github");
        let r2 = repo.clone();
        let issues = match tokio::task::spawn_blocking(move || github::list_issues(&cfg, &r2)).await.unwrap_or_else(|e| Err(e.to_string())) {
            Ok(i) => i,
            Err(e) => return Ok(Err(e)),
        };
        let rows = self.c().query(&format!("SELECT {TICKET_COLS} FROM tickets t WHERE t.project_id = $1"), &[&project]).await?;
        let tickets: Vec<Ticket> = rows.iter().map(ticket_from).collect();
        let refs: Vec<&Ticket> = tickets.iter().collect();
        let (mut imported, mut pulled, mut pushed, mut errors) = (0, 0, 0, vec![]);
        for action in plan_sync(&refs, &issues) {
            match action {
                SyncAction::Import(issue) => {
                    let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                    let status = if issue.open { Status::Todo } else { Status::Done };
                    t.execute(
                        "INSERT INTO tickets (project_id, title, body, tags, status, github_number, github_url, github_state, github_synced_at, position)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), COALESCE((SELECT max(position) FROM tickets WHERE project_id = $1), 0) + 1)",
                        &[&project, &issue.title, &issue.body, &tags_from_labels(&issue.labels), &status.as_str(), &issue.number, &issue.url,
                          &if issue.open { "open" } else { "closed" }],
                    )
                    .await?;
                    t.commit().await?;
                    imported += 1;
                }
                SyncAction::Pull { ticket, issue } => {
                    let current = tickets.iter().find(|t| t.id == ticket).map(|t| t.status).unwrap_or(Status::Todo);
                    let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                    t.execute(
                        "UPDATE tickets SET title = $2, body = $3, tags = $4, status = $5, github_url = $6, github_state = $7, github_synced_at = now()
                         WHERE id = $1",
                        &[&ticket, &issue.title, &issue.body, &tags_from_labels(&issue.labels), &status_from_issue(current, issue.open).as_str(),
                          &issue.url, &if issue.open { "open" } else { "closed" }],
                    )
                    .await?;
                    t.commit().await?;
                    pulled += 1;
                }
                SyncAction::Push { ticket, .. } => match self.push_ticket(ticket).await? {
                    Ok(()) => pushed += 1,
                    Err(e) => errors.push(e),
                },
            }
        }
        let mut summary = format!("{name} ↔ {repo} : {imported} importé(s), {pulled} mis à jour depuis GitHub, {pushed} envoyé(s) vers GitHub");
        if !errors.is_empty() {
            summary.push_str(&format!(" — {} erreur(s) : {}", errors.len(), errors[0]));
        }
        Ok(Ok(summary))
    }
}

const TICKET_COLS: &str = "t.id, t.project_id, t.number, t.title, t.body, t.status, t.tags, t.priority, t.github_number, t.github_url,
  t.github_state, t.github_synced_at, t.created_at, t.updated_at, t.done_at";
const TICKET_COLS_N: usize = 15;

fn ticket_from(r: &Row) -> Ticket {
    Ticket {
        id: r.get(0),
        project_id: r.get(1),
        number: r.get(2),
        title: r.get(3),
        body: r.get(4),
        status: Status::parse(r.get::<_, &str>(5)),
        tags: r.get(6),
        priority: r.get(7),
        github_number: r.get(8),
        github_url: r.get(9),
        github_state: r.get(10),
        github_synced_at: r.get(11),
        created_at: r.get(12),
        updated_at: r.get(13),
        done_at: r.get(14),
    }
}

async fn load_board(c: &Client) -> Res<Board> {
    let projects = c
        .query("SELECT id, name, description, github_repo FROM projects ORDER BY position, lower(name)", &[])
        .await?
        .iter()
        .map(|r| Project { id: r.get(0), name: r.get(1), description: r.get(2), github_repo: r.get(3) })
        .collect();
    let tickets = c
        .query(&format!("SELECT {TICKET_COLS} FROM tickets t ORDER BY t.project_id, t.priority DESC, t.position, t.number"), &[])
        .await?
        .iter()
        .map(ticket_from)
        .collect();
    let reports = c
        .query("SELECT r.ticket_id, r.note_id, n.path FROM ticket_reports r JOIN notes n ON n.id = r.note_id ORDER BY r.created_at", &[])
        .await?
        .iter()
        .map(|r| ReportLink { ticket_id: r.get(0), note_id: r.get(1), path: r.get(2) })
        .collect();
    Ok(Board { projects, tickets, reports })
}



struct Emitter {
    out: std::sync::mpsc::Sender<Msg>,
    notify: Arc<dyn Fn() + Send + Sync>,
}
impl Emitter {
    fn emit(&self, msg: Msg) {
        let _ = self.out.send(msg);
        (self.notify)();
    }
}

/// Opens a transaction tagged with this instance's client id
/// (read by the triggers to fill `updated_by`, so each instance can ignore its own echoes).
async fn tagged_tx<'a>(client: &'a mut Client, client_id: &str) -> Res<tokio_postgres::Transaction<'a>> {
    let t = client.transaction().await?;
    t.execute("SELECT set_config('app.client_id', $1, true)", &[&client_id]).await?;
    Ok(t)
}

const NOTE_COLS: &str = "id, path, content, version, updated_by, updated_at";
const PREFIXED: &str = "(lower(path) = lower($1) OR left(lower(path), length($1) + 1) = lower($1) || '/')";
const RESOLVE_SQL: &str = "SELECT id, path, content, version, updated_by, updated_at FROM notes
  WHERE lower(path) = lower($1) OR lower(regexp_replace(path, '^.*/', '')) = lower($1)
  ORDER BY (lower(path) = lower($1)) DESC, length(path) LIMIT 1";

fn note_from(r: &Row) -> Note {
    Note { id: r.get(0), path: r.get(1), content: r.get(2), version: r.get(3), updated_by: r.get(4), updated_at: r.get(5) }
}

fn is_unique_violation(e: &tokio_postgres::Error) -> bool {
    e.code() == Some(&tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
}

fn pg_err(e: &tokio_postgres::Error) -> String {
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

/// After a rename, rewrites `[[old]]`, `[[old|…]]`, `[[old#…]]` in every note linking to it.
async fn relink(t: &mut tokio_postgres::Transaction<'_>, id: Uuid, old: &str, new: &str) -> Res<usize> {
    let old_base = basename(old).to_string();
    // Use the full path if the new basename becomes ambiguous
    let same: i64 = t
        .query_one("SELECT count(*) FROM notes WHERE lower(regexp_replace(path, '^.*/', '')) = lower($1) AND id <> $2", &[&basename(new), &id])
        .await?
        .get(0);
    let target = if same > 0 { new.to_string() } else { basename(new).to_string() };
    let rows = t
        .query(
            "SELECT DISTINCT n.id, n.content FROM links l JOIN notes n ON n.id = l.source_id
             WHERE lower(l.target) = lower($1) OR lower(l.target) = lower($2)",
            &[&old, &old_base],
        )
        .await?;
    let mut changed = 0;
    for r in rows {
        let (nid, content): (Uuid, String) = (r.get(0), r.get(1));
        let next = rewrite_links(&content, &[old, &old_base], &target);
        if next != content {
            t.execute("UPDATE notes SET content = $1 WHERE id = $2", &[&next, &nid]).await?;
            changed += 1;
        }
    }
    Ok(changed)
}

pub fn rewrite_links(content: &str, from: &[&str], to: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(pos) = rest.find("[[") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        let end = after.find([']', '|', '#', '\n']).unwrap_or(after.len());
        let name = after[..end].trim();
        let closes = after[end..].starts_with(['|', '#', ']']);
        if closes && from.iter().any(|f| f.to_lowercase() == name.to_lowercase()) {
            out.push_str("[[");
            out.push_str(to);
            rest = &after[end..];
        } else {
            out.push_str("[[");
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

// ------------------------------------------------------------------ connections

fn wants_tls(url: &str) -> bool {
    let u = url.to_lowercase();
    ["sslmode=require", "sslmode=verify-ca", "sslmode=verify-full"].iter().any(|m| u.contains(m))
}

fn tls() -> tokio_postgres_rustls::MakeRustlsConnect {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()
        .expect("tls versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    tokio_postgres_rustls::MakeRustlsConnect::new(config)
}

/// Where LISTEN notifications are forwarded (channel + repaint callback).
type EventSink = (std::sync::mpsc::Sender<Msg>, Arc<dyn Fn() + Send + Sync>);

fn drive<S, T>(conn: Connection<S, T>, events: Option<EventSink>) -> tokio::task::JoinHandle<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    T: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut conn = conn;
        let mut stream = futures_util::stream::poll_fn(move |cx| conn.poll_message(cx));
        while let Some(msg) = stream.next().await {
            match msg {
                Ok(AsyncMessage::Notification(n)) => {
                    if let Some((out, notify)) = &events {
                        match serde_json::from_str::<VaultEvent>(n.payload()) {
                            Ok(ev) => {
                                let _ = out.send(Msg::Event(ev));
                                notify();
                            }
                            Err(e) => eprintln!("[db] bad notification payload: {e}"),
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("[db] connection error: {e}");
                    break;
                }
            }
        }
    })
}

async fn open_client(url: &str) -> Res<Client> {
    connect_with(url, None).await.map(|(c, _)| c)
}

async fn connect_with(
    url: &str,
    events: Option<EventSink>,
) -> Res<(Client, tokio::task::JoinHandle<()>)> {
    let mut config: tokio_postgres::Config = url.parse()?;
    config.connect_timeout(Duration::from_secs(10));
    if wants_tls(url) {
        let (c, conn) = config.connect(tls()).await?;
        Ok((c, drive(conn, events)))
    } else {
        let (c, conn) = config.connect(NoTls).await?;
        Ok((c, drive(conn, events)))
    }
}

/// Keeps a LISTEN connection alive forever, reconnecting with exponential backoff.
async fn listen_loop(url: String, out: std::sync::mpsc::Sender<Msg>, notify: Arc<dyn Fn() + Send + Sync>) {
    let mut attempt = 0u32;
    let mut first = true;
    loop {
        match connect_with(&url, Some((out.clone(), notify.clone()))).await {
            Ok((client, driver)) => match client.batch_execute("LISTEN vault_events").await {
                Ok(()) => {
                    attempt = 0;
                    let _ = out.send(Msg::Live(true));
                    if !first {
                        let _ = out.send(Msg::Resync);
                    }
                    notify();
                    first = false;
                    let _ = driver.await; // returns when the connection drops
                    drop(client);
                }
                Err(e) => eprintln!("[db] LISTEN failed: {e}"),
            },
            Err(e) => eprintln!("[db] listener connection failed: {}", pg_err(&e)),
        }
        let _ = out.send(Msg::Live(false));
        notify();
        let delay = Duration::from_millis((500u64 << attempt.min(6)).min(30_000));
        attempt += 1;
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite_links;

    #[test]
    fn rewrite() {
        let s = "a [[Old]] b [[old|alias]] c [[Dir/Old#h]] d [[Older]] [[x]]";
        assert_eq!(rewrite_links(s, &["Dir/Old", "Old"], "New"), "a [[New]] b [[New|alias]] c [[New#h]] d [[Older]] [[x]]");
    }
}
