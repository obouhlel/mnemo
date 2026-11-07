//! Synchronisation des tickets avec les issues GitHub (publication, push, synchronisation).
//! Le `Result` externe est une erreur de base de données, l'interne une erreur GitHub (affichée à l'utilisateur).

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

    pub(super) async fn mark_synced(&mut self, id: Uuid, issue: &RemoteIssue) -> Res<()> {
        let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
        t.execute(
            "UPDATE tickets SET github_number = $2, github_url = $3, github_state = $4, github_synced_at = now() WHERE id = $1",
            &[&id, &issue.number, &issue.url, &remote_state(issue)],
        )
        .await?;
        t.commit().await
    }

    pub(super) async fn push_ticket(&mut self, id: Uuid) -> Res<Result<(), String>> {
        let Some((tk, repo)) = self.ticket_with_repo(id).await? else {
            return Ok(Err("Projet sans dépôt GitHub".into()));
        };
        let Some(number) = tk.github_number else { return Ok(Err("Ticket non publié sur GitHub".into())) };
        let cfg = self.github.clone().expect("github");
        let labels: Vec<String> = tk.tags.iter().map(|t| tag_to_label(t)).collect();
        // une pull request n'est jamais fermée depuis Mnemo (on n'envoie que titre, description et labels)
        let state = if tk.github_pr { None } else { Some(tk.status != Status::Done) };
        let r = tokio::task::spawn_blocking(move || {
            github::update_issue(&cfg, &repo, number, &tk.title, &tk.body, state, &labels)
        })
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

    pub(super) async fn publish_ticket(&mut self, id: Uuid) -> Res<Result<i32, String>> {
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
            if done {
                github::update_issue(&cfg, &repo, issue.number, &tk.title, &tk.body, Some(false), &labels)
            } else {
                Ok(issue)
            }
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
        let (issues, labels) = match fetched {
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
                    t.execute(
                        "INSERT INTO tickets (project_id, title, body, tags, status, github_number, github_url, github_state, github_pr, github_synced_at, position)
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, now(), COALESCE((SELECT max(position) FROM tickets WHERE project_id = $1), 0) + 1)",
                        &[&project, &issue.title, &issue.body, &tags_from_labels(&issue.labels), &status.as_str(), &issue.number, &issue.url,
                          &remote_state(&issue), &issue.pull],
                    )
                    .await?;
                    t.commit().await?;
                    imported += 1;
                }
                SyncAction::Pull { ticket, issue } => {
                    let current = tickets.iter().find(|t| t.id == ticket).map(|t| t.status).unwrap_or(Status::Todo);
                    let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                    t.execute(
                        "UPDATE tickets SET title = $2, body = $3, tags = $4, status = $5, github_url = $6, github_state = $7, github_pr = $8,
                           github_synced_at = now()
                         WHERE id = $1",
                        &[&ticket, &issue.title, &issue.body, &tags_from_labels(&issue.labels), &status_from_remote(current, &issue).as_str(),
                          &issue.url, &remote_state(&issue), &issue.pull],
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
        let mut summary = format!(
            "{name} ↔ {repo} : {} issue(s) et {prs} pull request(s) sur GitHub — {imported} importé(s), {pulled} mis à jour, {pushed} envoyé(s)",
            issues.len() - prs
        );
        if !errors.is_empty() {
            summary.push_str(&format!(" — {} erreur(s) : {}", errors.len(), errors[0]));
        }
        Ok(Ok(summary))
    }
}
