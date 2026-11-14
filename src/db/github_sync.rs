//! Synchronisation des tickets avec les issues GitHub (publication, push, synchronisation).
//! Le `Result` externe est une erreur de base de données, l'interne une erreur GitHub (affichée à l'utilisateur).
//!
//! Un ticket lié garde dans `github_base` l'issue telle que vue à la dernière synchro : `board::merge_ticket`
//! en déduit, champ par champ, ce qui a changé de chaque côté. Rien n'est envoyé sans avoir lu l'issue d'abord.

use super::*;

impl Worker {
    /// Commandes GitHub (publier un ticket, synchroniser un projet).
    pub(super) async fn github_cmd(&mut self, cmd: Cmd, em: &Emitter) -> Res<()> {
        match cmd {
            Cmd::GithubPublish(id) => match self.github.is_some() {
                false => {
                    em.emit(Msg::GithubFailed(None, "Configurez d'abord un token GitHub (Paramètres › GitHub)".into()))
                }
                true => match self.publish_ticket(id).await? {
                    Ok(n) => em.emit(Msg::GithubDone(None, format!("Issue #{n} créée sur GitHub"))),
                    Err(e) => em.emit(Msg::GithubFailed(None, e)),
                },
            },
            Cmd::GithubSync(project) => match self.github.is_some() {
                false => em.emit(Msg::GithubFailed(
                    Some(project),
                    "Configurez d'abord un token GitHub (Paramètres › GitHub)".into(),
                )),
                true => match self.sync_project(project).await? {
                    Ok(summary) => em.emit(Msg::GithubDone(Some(project), summary)),
                    Err(e) => em.emit(Msg::GithubFailed(Some(project), e)),
                },
            },
            _ => unreachable!("commande routée ailleurs par Worker::handle"),
        }
        Ok(())
    }

    /// Synchronise un projet qu'on vient de lier à un dépôt (si un token est configuré).
    pub(super) async fn import_now(&mut self, project: Uuid, em: &Emitter) -> Res<()> {
        if self.github.is_none() {
            return Ok(());
        }
        match self.sync_project(project).await? {
            Ok(summary) => em.emit(Msg::GithubDone(Some(project), summary)),
            Err(e) => em.emit(Msg::GithubFailed(Some(project), e)),
        }
        Ok(())
    }

    pub(super) async fn ticket_with_repo(&self, id: Uuid) -> Res<Option<(Ticket, String)>> {
        let row = self
            .conn()
            .query_opt(&format!("SELECT {TICKET_COLS}, p.github_repo FROM tickets t JOIN projects p ON p.id = t.project_id WHERE t.id = $1"), &[&id])
            .await?;
        Ok(row.and_then(|r| r.get::<_, Option<String>>(TICKET_COLS_N).map(|repo| (ticket_from(&r), repo))))
    }

    /// Lie le ticket à l'issue qu'on vient de créer : numéro, adresse, état, et l'issue comme base de fusion.
    pub(super) async fn mark_synced(&mut self, id: Uuid, issue: &RemoteIssue) -> Res<()> {
        let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
        t.execute(
            "UPDATE tickets SET github_number = $2, github_url = $3, github_state = $4, github_pr = $5, github_base = $6,
               github_synced_at = now()
             WHERE id = $1",
            &[&id, &issue.number, &issue.url, &remote_state(issue), &issue.pull, &IssueBase::of(issue).to_json()],
        )
        .await?;
        t.commit().await
    }

    /// Applique une fusion : envoie à GitHub les champs modifiés localement, puis écrit dans le ticket ceux qui
    /// viennent de GitHub et la nouvelle base. Si l'envoi échoue, la base reste l'issue lue : il sera retenté.
    /// Le ticket n'est touché que s'il désigne toujours cette issue (projet déplacé ou dépôt changé entre-temps).
    async fn apply_merge(
        &mut self,
        tk: &Ticket,
        repo: &str,
        issue: RemoteIssue,
        merged: Merged,
    ) -> Res<Result<(), String>> {
        let mut seen = issue;
        let mut failed = None;
        if !merged.push.is_empty() {
            let (cfg, repo, patch, number) =
                (self.github.clone().expect("github"), repo.to_string(), merged.push, seen.number);
            match tokio::task::spawn_blocking(move || github::update_issue(&cfg, &repo, number, &patch))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
            {
                Ok(after) => seen = after,
                Err(e) => failed = Some(e),
            }
        }
        let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
        t.execute(
            "UPDATE tickets SET title = COALESCE($4, title), body = COALESCE($5, body), tags = COALESCE($6, tags),
               status = COALESCE($7, status), github_url = $8, github_state = $9, github_pr = $10, github_base = $11,
               github_synced_at = now()
             WHERE id = $1 AND project_id = $2 AND github_number = $3",
            &[
                &tk.id,
                &tk.project_id,
                &seen.number,
                &merged.title,
                &merged.body,
                &merged.tags,
                &merged.status.map(|s| s.as_str()),
                &seen.url,
                &remote_state(&seen),
                &seen.pull,
                &IssueBase::of(&seen).to_json(),
            ],
        )
        .await?;
        t.commit().await?;
        Ok(failed.map_or(Ok(()), Err))
    }

    /// Met l'issue d'un ticket qu'on vient de modifier en accord avec lui : l'issue est lue d'abord, et seuls
    /// les champs modifiés localement depuis la dernière synchro sont envoyés (ce qui a changé sur GitHub
    /// entre-temps est repris dans le ticket, par exemple une fermeture).
    pub(super) async fn push_ticket(&mut self, id: Uuid) -> Res<Result<(), String>> {
        let Some((tk, repo)) = self.ticket_with_repo(id).await? else {
            return Ok(Err("Projet sans dépôt GitHub".into()));
        };
        let Some(number) = tk.github_number else { return Ok(Err("Ticket non publié sur GitHub".into())) };
        let (cfg, r2) = (self.github.clone().expect("github"), repo.clone());
        let issue = match tokio::task::spawn_blocking(move || github::get_issue(&cfg, &r2, number))
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        {
            Ok(i) => i,
            Err(e) => return Ok(Err(e)),
        };
        let merged = merge_ticket(&tk, &issue);
        self.apply_merge(&tk, &repo, issue, merged).await
    }

    pub(super) async fn publish_ticket(&mut self, id: Uuid) -> Res<Result<i32, String>> {
        let Some((tk, repo)) = self.ticket_with_repo(id).await? else {
            return Ok(Err("Associez d'abord un dépôt GitHub au projet".into()));
        };
        if let Some(n) = tk.github_number {
            return Ok(Err(format!("Déjà publié (issue #{n})")));
        }
        let cfg = self.github.clone().expect("github");
        let labels = labels_from_tags(&tk.tags);
        let (c, r, title, body) = (cfg.clone(), repo.clone(), tk.title.clone(), tk.body.clone());
        let created = tokio::task::spawn_blocking(move || github::create_issue(&c, &r, &title, &body, &labels))
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
        let issue = match created {
            Ok(i) => i,
            Err(e) => return Ok(Err(e)),
        };
        // le numéro est enregistré tout de suite : si la fermeture ci-dessous échoue, le ticket reste lié à son
        // issue (ouverte) et la prochaine synchronisation la ferme, au lieu d'en créer une deuxième
        self.mark_synced(id, &issue).await?;
        if tk.status == Status::Done {
            let number = issue.number;
            let close = IssuePatch { open: Some(false), ..Default::default() };
            let closed = tokio::task::spawn_blocking(move || github::update_issue(&cfg, &repo, number, &close))
                .await
                .unwrap_or_else(|e| Err(e.to_string()));
            match closed {
                Ok(issue) => self.mark_synced(id, &issue).await?,
                Err(e) => {
                    return Ok(Err(format!(
                        "Issue #{number} créée, mais pas encore fermée ({e}) : elle le sera à la prochaine synchronisation"
                    )));
                }
            }
        }
        Ok(Ok(issue.number))
    }

    pub(super) async fn sync_project(&mut self, project: Uuid) -> Res<Result<String, String>> {
        let Some(row) =
            self.conn().query_opt("SELECT name, github_repo FROM projects WHERE id = $1", &[&project]).await?
        else {
            return Ok(Err("Projet introuvable".into()));
        };
        let (name, repo): (String, Option<String>) = (row.get(0), row.get(1));
        let Some(repo) = repo else { return Ok(Err(format!("Le projet « {name} » n'a pas de dépôt GitHub"))) };
        let cfg = self.github.clone().expect("github");
        let r2 = repo.clone();
        let fetched = tokio::task::spawn_blocking(move || {
            Ok::<_, String>((github::list_issues(&cfg, &r2)?, github::list_labels(&cfg, &r2)?))
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        let ((issues, truncated), labels) = match fetched {
            Ok(x) => x,
            Err(e) => return Ok(Err(e)),
        };
        // labels du dépôt (nom + couleur) : les tags reprennent les mêmes, avec la même couleur
        let labels_json = serde_json::Value::from(
            labels.iter().map(|(n, c)| serde_json::json!([n, c])).collect::<Vec<serde_json::Value>>(),
        );
        self.conn()
            .execute(
                "UPDATE projects SET github_labels = $2 WHERE id = $1 AND github_labels IS DISTINCT FROM $2",
                &[&project, &labels_json],
            )
            .await?;
        let prs = issues.iter().filter(|i| i.pull).count();
        let rows = self
            .conn()
            .query(&format!("SELECT {TICKET_COLS} FROM tickets t WHERE t.project_id = $1"), &[&project])
            .await?;
        let tickets: Vec<Ticket> = rows.iter().map(ticket_from).collect();
        let refs: Vec<&Ticket> = tickets.iter().collect();
        let (mut imported, mut pulled, mut pushed, mut errors) = (0, 0, 0, vec![]);
        for action in plan_sync(&refs, &issues) {
            match action {
                SyncAction::Import(issue) => {
                    let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                    let status = status_from_remote(Status::Todo, &issue);
                    // seulement si le projet est toujours lié à ce dépôt (il a pu changer pendant la lecture des issues)
                    imported += t.execute(
                        "INSERT INTO tickets (project_id, title, body, tags, status, github_number, github_url, github_state, github_pr,
                           github_base, github_synced_at, position)
                         SELECT $1::uuid, $2::text, $3::text, $4::text[], $5::text, $6::int4, $7::text, $8::text, $9::bool, $10::jsonb, now(),
                                COALESCE((SELECT max(position) FROM tickets WHERE project_id = $1), 0) + 1
                         WHERE EXISTS (SELECT 1 FROM projects WHERE id = $1 AND lower(github_repo) = lower($11::text))
                         ON CONFLICT DO NOTHING",
                        &[&project, &issue.title, &issue.body, &tags_from_labels(&issue.labels), &status.as_str(), &issue.number, &issue.url,
                          &remote_state(&issue), &issue.pull, &IssueBase::of(&issue).to_json(), &repo],
                    )
                    .await?;
                    t.commit().await?;
                }
                SyncAction::Update { ticket, issue, merged } => {
                    let Some(tk) = tickets.iter().find(|t| t.id == ticket) else { continue };
                    let (pulls, pushes) = (merged.pulls(), !merged.push.is_empty());
                    match self.apply_merge(tk, &repo, issue, merged).await? {
                        Ok(()) => {
                            pulled += u64::from(pulls);
                            pushed += u64::from(pushes);
                        }
                        Err(e) => errors.push(e),
                    }
                }
            }
        }
        let mut summary = format!(
            "{name} ↔ {repo} : {} issue(s) et {prs} pull request(s) sur GitHub — {imported} importé(s), {pulled} mis à jour, {pushed} envoyé(s)",
            issues.len() - prs
        );
        if truncated {
            summary.push_str(&format!(
                " — liste limitée aux {} premières issues et pull requests, les autres ne sont pas synchronisées",
                issues.len()
            ));
        }
        if !errors.is_empty() {
            summary.push_str(&format!(" — {} erreur(s) : {}", errors.len(), errors[0]));
        }
        Ok(Ok(summary))
    }
}

/// Test de bout en bout avec une vraie base (`MNEMO_TEST_PG`, comme `sync_tests.rs`) et un faux serveur GitHub
/// local : ce que Mnemo envoie réellement à l'API quand un ticket ou un projet change.
#[cfg(test)]
mod tests {
    use crate::board::{Status, Ticket, TicketPatch};
    use crate::db::{Cmd, DbHandle, Msg};
    use crate::github::GhConfig;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::mpsc::Receiver;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    /// Faux GitHub : issues par dépôt, journal des écritures reçues.
    #[derive(Default)]
    struct Fake {
        issues: Vec<(String, Value)>,
        /// (méthode, chemin, corps)
        writes: Vec<(String, String, Value)>,
        fail_next_patch: bool,
        clock: i64,
    }

    impl Fake {
        fn stamp(&mut self) -> String {
            self.clock += 1;
            (chrono::DateTime::from_timestamp(1_800_000_000 + self.clock, 0).unwrap()).to_rfc3339()
        }
        fn add(&mut self, repo: &str, title: &str, labels: &[&str]) -> Value {
            let number = self.issues.iter().filter(|(r, _)| r == repo).count() + 1;
            let labels: Vec<Value> = labels.iter().map(|l| json!({ "name": l })).collect();
            let issue = json!({"number": number, "title": title, "body": "", "state": "open", "labels": labels,
                               "html_url": format!("https://example.invalid/{repo}/issues/{number}"), "updated_at": self.stamp()});
            self.issues.push((repo.to_string(), issue.clone()));
            issue
        }
        fn issue(&mut self, repo: &str, number: i64) -> Option<&mut Value> {
            self.issues.iter_mut().find(|(r, i)| r == repo && i["number"] == number).map(|(_, i)| i)
        }
        /// Modification faite « sur GitHub » par quelqu'un d'autre.
        fn edit(&mut self, repo: &str, number: i64, key: &str, value: Value) {
            let stamp = self.stamp();
            let i = self.issue(repo, number).unwrap();
            i[key] = value;
            i["updated_at"] = json!(stamp);
        }

        fn respond(&mut self, method: &str, target: &str, body: Value) -> (u16, Value) {
            let (path, query) = target.split_once('?').unwrap_or((target, ""));
            let parts: Vec<&str> = path.trim_matches('/').split('/').collect();
            if method != "GET" {
                self.writes.push((method.to_string(), path.to_string(), body.clone()));
            }
            match (method, &parts[..]) {
                ("GET", ["repos", o, r, "issues"]) => {
                    let repo = format!("{o}/{r}");
                    let first = query.split('&').any(|q| q == "page=1");
                    let all: Vec<Value> =
                        self.issues.iter().filter(|(x, _)| *x == repo && first).map(|(_, i)| i.clone()).collect();
                    (200, json!(all))
                }
                ("GET", ["repos", _, _, "labels"]) => (200, json!([])),
                ("GET", ["repos", o, r, "issues", n]) => match self.issue(&format!("{o}/{r}"), n.parse().unwrap()) {
                    Some(i) => (200, i.clone()),
                    None => (404, json!({"message": "Not Found"})),
                },
                ("POST", ["repos", o, r, "issues"]) => {
                    let labels: Vec<&str> =
                        body["labels"].as_array().unwrap().iter().filter_map(|l| l.as_str()).collect();
                    (201, self.add(&format!("{o}/{r}"), body["title"].as_str().unwrap(), &labels))
                }
                ("PATCH", ["repos", o, r, "issues", n]) => {
                    if std::mem::take(&mut self.fail_next_patch) {
                        return (500, json!({"message": "panne"}));
                    }
                    let stamp = self.stamp();
                    let i = self.issue(&format!("{o}/{r}"), n.parse().unwrap()).unwrap();
                    for (k, v) in body.as_object().unwrap() {
                        i[k] = match k.as_str() {
                            "labels" => v.as_array().unwrap().iter().map(|l| json!({ "name": l })).collect(),
                            _ => v.clone(),
                        };
                    }
                    i["updated_at"] = json!(stamp);
                    (200, i.clone())
                }
                _ => (404, json!({"message": "Not Found"})),
            }
        }
    }

    /// Sert le faux GitHub sur un port local ; renvoie l'adresse de l'API.
    fn serve(fake: Arc<Mutex<Fake>>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut it = line.split_whitespace();
                let (method, target) = (it.next().unwrap_or("").to_string(), it.next().unwrap_or("").to_string());
                let mut len = 0;
                loop {
                    let mut h = String::new();
                    reader.read_line(&mut h).unwrap();
                    if h.trim().is_empty() {
                        break;
                    }
                    if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; len];
                reader.read_exact(&mut body).unwrap();
                let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
                let (code, out) = fake.lock().unwrap().respond(&method, &target, body);
                let out = out.to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}",
                    out.len()
                );
            }
        });
        api
    }

    struct App {
        db: DbHandle,
        rx: Receiver<Msg>,
    }

    impl App {
        fn wait<T>(&self, what: &str, mut f: impl FnMut(Msg) -> Option<T>) -> T {
            let end = Instant::now() + Duration::from_secs(20);
            loop {
                let left =
                    end.checked_duration_since(Instant::now()).unwrap_or_else(|| panic!("délai dépassé : {what}"));
                let m = self.rx.recv_timeout(left).unwrap_or_else(|_| panic!("délai dépassé : {what}"));
                if let Msg::GithubFailed(_, e) = &m
                    && !what.contains("échec")
                {
                    panic!("{what} : erreur GitHub inattendue : {e}");
                }
                if let Some(v) = f(m) {
                    return v;
                }
            }
        }
        /// Les commandes sont traitées dans l'ordre : le tableau reçu reflète toutes celles envoyées avant.
        fn tickets(&self) -> Vec<Ticket> {
            self.db.send(Cmd::LoadBoard);
            self.wait("tableau", |m| if let Msg::Board(b) = m { Some(b.tickets) } else { None })
        }
        fn ticket(&self, title: &str) -> Ticket {
            self.tickets().into_iter().find(|t| t.title == title).unwrap_or_else(|| panic!("ticket « {title} »"))
        }
        fn project(&self, name: &str, repo: &str) -> Uuid {
            self.db.send(Cmd::CreateProject { name: name.into(), description: String::new(), repo: Some(repo.into()) });
            self.wait("import", |m| if let Msg::GithubDone(Some(id), _) = m { Some(id) } else { None })
        }
        fn sync(&self, project: Uuid) -> String {
            self.db.send(Cmd::GithubSync(project));
            self.wait("synchronisation", |m| if let Msg::GithubDone(_, s) = m { Some(s) } else { None })
        }
        fn patch(&self, id: Uuid, patch: TicketPatch) {
            self.db.send(Cmd::UpdateTicket { id, patch });
        }
    }

    #[test]
    fn tickets_and_issues_stay_consistent() {
        let Some(server) = std::env::var("MNEMO_TEST_PG").ok().filter(|s| !s.trim().is_empty()) else { return };
        let name = format!("mnemo_test_{}", Uuid::new_v4().simple());
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let admin = format!("{}/postgres", server.trim_end_matches('/'));
        rt.block_on(async {
            let (client, conn) = tokio_postgres::connect(&admin, tokio_postgres::NoTls).await.expect("MNEMO_TEST_PG");
            tokio::spawn(conn);
            client.batch_execute(&format!("CREATE DATABASE {name}")).await.expect("create database");
        });
        let fake = Arc::new(Mutex::new(Fake::default()));
        fake.lock().unwrap().add("o/r", "Un", &["enhancement", "help wanted"]);
        fake.lock().unwrap().add("o/other", "Autre", &[]);
        let api = serve(fake.clone());
        let writes = || std::mem::take(&mut fake.lock().unwrap().writes);

        let (tx, rx) = std::sync::mpsc::channel();
        let db = crate::db::spawn(Uuid::new_v4().to_string(), tx, || {}, crate::local::shared::new_shared());
        let app = App { db, rx };
        app.db.send(Cmd::Connect(format!("{}/{name}", server.trim_end_matches('/'))));
        app.wait("connexion", |m| matches!(m, Msg::Connected).then_some(()));
        app.db.send(Cmd::SetGithub(Some(GhConfig { token: "jeton".into(), api })));

        // import : le label `enhancement` devient le tag intégré, et repart tel quel (G5)
        let p = app.project("P", "o/r");
        let un = app.ticket("Un");
        assert_eq!(
            (un.github_number, un.tags.clone()),
            (Some(1), vec!["Amélioration".to_string(), "help wanted".into()])
        );
        assert!(app.sync(p).contains("0 importé(s), 0 mis à jour, 0 envoyé(s)"));
        assert!(writes().is_empty());

        // G2 : issue fermée sur GitHub, puis faute corrigée dans le titre du ticket → seul le titre est envoyé,
        // l'issue reste fermée et le ticket passe à « terminé »
        fake.lock().unwrap().edit("o/r", 1, "state", json!("closed"));
        app.patch(un.id, TicketPatch { title: Some("Un corrigé".into()), ..Default::default() });
        let un = app.ticket("Un corrigé");
        assert_eq!(
            writes(),
            vec![("PATCH".to_string(), "/repos/o/r/issues/1".to_string(), json!({"title": "Un corrigé"}))]
        );
        assert_eq!((un.status, un.github_state.as_deref()), (Status::Done, Some("closed")));
        assert_eq!(fake.lock().unwrap().issue("o/r", 1).unwrap()["state"], "closed");

        // G2 : priorité (invisible sur GitHub) → rien à envoyer, ni tout de suite ni à la synchro
        app.patch(un.id, TicketPatch { priority: Some(3), ..Default::default() });
        // description modifiée sur GitHub + tag ajouté localement : chacun reçoit le champ de l'autre
        fake.lock().unwrap().edit("o/r", 1, "body", json!("depuis GitHub"));
        app.patch(
            un.id,
            TicketPatch {
                tags: Some(vec!["Amélioration".into(), "help wanted".into(), "Bug".into()]),
                ..Default::default()
            },
        );
        let un = app.ticket("Un corrigé");
        assert_eq!(
            writes(),
            vec![(
                "PATCH".to_string(),
                "/repos/o/r/issues/1".to_string(),
                json!({"labels": ["enhancement", "help wanted", "bug"]})
            )]
        );
        assert_eq!((un.body.as_str(), un.priority, un.status), ("depuis GitHub", 3, Status::Done));
        assert!(app.sync(p).contains("0 mis à jour, 0 envoyé(s)"));
        assert!(writes().is_empty());

        // G6 : ticket terminé publié, la fermeture échoue → le ticket garde son numéro d'issue, la synchro
        // suivante ferme l'issue sans en créer une deuxième
        app.db.send(Cmd::CreateTicket { project_id: p, title: "Fini".into(), tags: vec![], status: Status::Done });
        let fini = app.ticket("Fini");
        fake.lock().unwrap().fail_next_patch = true;
        app.db.send(Cmd::GithubPublish(fini.id));
        let e = app.wait("échec de la fermeture", |m| if let Msg::GithubFailed(_, e) = m { Some(e) } else { None });
        assert!(e.contains("Issue #2 créée"), "{e}");
        assert_eq!(app.ticket("Fini").github_number, Some(2));
        writes();
        assert!(app.sync(p).contains("1 envoyé(s)"));
        assert_eq!(
            writes(),
            vec![("PATCH".to_string(), "/repos/o/r/issues/2".to_string(), json!({"state": "closed"}))]
        );
        assert_eq!(fake.lock().unwrap().issues.iter().filter(|(r, _)| r == "o/r").count(), 2);

        // G1 / G3 : ticket déplacé vers un projet lié à un autre dépôt → nouveau numéro, lien effacé, et
        // l'issue #1 de l'autre dépôt n'est jamais modifiée
        let q = app.project("Q", "o/other");
        assert_eq!(app.ticket("Autre").number, 1);
        app.patch(un.id, TicketPatch { project_id: Some(q), title: Some("Déplacé".into()), ..Default::default() });
        let moved = app.ticket("Déplacé");
        assert_eq!((moved.project_id, moved.number, moved.github_number, moved.github_synced_at), (q, 2, None, None));
        app.sync(q);
        app.patch(moved.id, TicketPatch { status: Some(Status::Todo), ..Default::default() });
        app.sync(q);
        assert!(writes().is_empty());
        // l'issue d'origine, qui n'a plus de ticket dans P, y est réimportée
        assert!(app.sync(p).contains("1 importé(s)"));

        // G1 : le dépôt du projet change → ses tickets sont déliés, puis les issues du nouveau dépôt importées
        app.db.send(Cmd::UpdateProject {
            id: p,
            name: "P".into(),
            description: String::new(),
            repo: Some("o/other".into()),
        });
        app.wait("import du nouveau dépôt", |m| matches!(m, Msg::GithubDone(Some(id), _) if id == p).then_some(()));
        let in_p: Vec<Ticket> = app.tickets().into_iter().filter(|t| t.project_id == p).collect();
        let linked: Vec<(&str, Option<i32>)> =
            in_p.iter().filter(|t| t.github_number.is_some()).map(|t| (t.title.as_str(), t.github_number)).collect();
        assert_eq!(linked, vec![("Autre", Some(1))]);
        assert!(in_p.iter().any(|t| t.title == "Fini" && t.github_number.is_none()));
        app.patch(app.ticket("Fini").id, TicketPatch { title: Some("Fini (bis)".into()), ..Default::default() });
        app.sync(p);
        assert!(writes().is_empty());

        rt.block_on(async {
            let (client, conn) = tokio_postgres::connect(&admin, tokio_postgres::NoTls).await.unwrap();
            tokio::spawn(conn);
            let _ = client.batch_execute(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")).await;
        });
    }
}
