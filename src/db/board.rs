//! Commandes du tableau de bord : projets, tickets et rapports (notes liées aux tickets).

use super::*;

impl Worker {
    /// Commandes projets / tickets / rapports.
    pub(super) async fn board_cmd(&mut self, cmd: Cmd, em: &Emitter) -> Res<()> {
        match cmd {
            Cmd::LoadBoard => {
                let board = load_board(self.conn()).await?;
                em.emit(Msg::Board(board));
            }
            Cmd::CreateProject { name, description, repo } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                match t
                    .query_one(
                        "INSERT INTO projects (name, description, github_repo, position)
                         VALUES ($1, $2, $3, COALESCE((SELECT max(position) FROM projects), 0) + 1) RETURNING id",
                        &[&name.trim(), &description, &repo],
                    )
                    .await
                {
                    Ok(row) => {
                        t.commit().await?;
                        if repo.is_some() {
                            self.import_now(row.get(0), em).await?;
                        }
                    }
                    Err(e) if is_unique_violation(&e) => {
                        em.emit(Msg::Error(format!("Le projet « {name} » existe déjà")))
                    }
                    Err(e) => return Err(e),
                }
            }
            Cmd::UpdateProject { id, name, description, repo } => {
                // dépôt changé ou retiré : le trigger `projects_repo_changed` délie les tickets des issues de
                // l'ancien dépôt dans la même transaction (ils redeviennent « non publiés »)
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                match t
                    .query_opt(
                        "UPDATE projects p SET name = $2, description = $3, github_repo = $4
                         FROM (SELECT github_repo AS old FROM projects WHERE id = $1) o
                         WHERE p.id = $1 RETURNING o.old",
                        &[&id, &name.trim(), &description, &repo],
                    )
                    .await
                {
                    Ok(row) => {
                        t.commit().await?;
                        // nouveau dépôt lié : on importe tout de suite ses issues et pull requests
                        let old: Option<String> = row.and_then(|r| r.get(0));
                        if repo.is_some() && repo != old {
                            self.import_now(id, em).await?;
                        }
                    }
                    Err(e) if is_unique_violation(&e) => {
                        em.emit(Msg::Error(format!("Le projet « {name} » existe déjà")))
                    }
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
                // changement de projet : le trigger `tickets_track` donne un numéro libre dans le projet d'arrivée
                // et, si le dépôt n'est pas le même, efface le lien GitHub (voir schema.sql)
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute(
                    "UPDATE tickets SET title = COALESCE($2, title), body = COALESCE($3, body), status = COALESCE($4, status),
                       tags = COALESCE($5, tags), priority = COALESCE($6, priority), project_id = COALESCE($7, project_id)
                     WHERE id = $1",
                    &[&id, &patch.title, &patch.body, &patch.status.map(|s| s.as_str()), &patch.tags, &patch.priority, &patch.project_id],
                )
                .await?;
                t.commit().await?;
                // keep the GitHub issue in step (title, body, labels, open/closed) : `push_ticket` lit d'abord
                // l'issue et n'envoie que ce qui a changé ici depuis la dernière synchro
                let touches_github =
                    patch.title.is_some() || patch.body.is_some() || patch.status.is_some() || patch.tags.is_some();
                if touches_github && self.github.is_some() {
                    let linked = self
                        .conn()
                        .query_opt(
                            "SELECT t.github_number FROM tickets t JOIN projects p ON p.id = t.project_id
                         WHERE t.id = $1 AND t.github_number IS NOT NULL AND p.github_repo IS NOT NULL",
                            &[&id],
                        )
                        .await?;
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
                t.execute(
                    "INSERT INTO ticket_reports (ticket_id, note_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    &[&ticket, &note],
                )
                .await?;
                t.commit().await?;
            }
            Cmd::UnlinkReport { ticket, note } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                t.execute("DELETE FROM ticket_reports WHERE ticket_id = $1 AND note_id = $2", &[&ticket, &note])
                    .await?;
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
                let row = t
                    .query_one(
                        &format!("INSERT INTO notes (path, content) VALUES ($1, $2) RETURNING {NOTE_COLS}"),
                        &[&candidate, &content],
                    )
                    .await?;
                let note = note_from(&row);
                t.execute("INSERT INTO ticket_reports (ticket_id, note_id) VALUES ($1, $2)", &[&ticket, &note.id])
                    .await?;
                t.commit().await?;
                em.emit(Msg::Created { note, open: true });
            }
            _ => unreachable!("commande routée ailleurs par Worker::handle"),
        }
        Ok(())
    }
}

pub(super) const TICKET_COLS: &str =
    "t.id, t.project_id, t.number, t.title, t.body, t.status, t.tags, t.priority, t.github_number, t.github_url,
  t.github_state, t.github_synced_at, t.created_at, t.updated_at, t.done_at, t.github_pr, t.content_changed_at,
  t.github_base";

pub(super) const TICKET_COLS_N: usize = 18;

pub(super) fn ticket_from(r: &Row) -> Ticket {
    Ticket {
        id: r.get(0),
        project_id: r.get(1),
        number: r.get(2),
        title: r.get(3),
        body: r.get(4),
        status: Status::parse(r.get::<_, &str>(5)),
        tags: canonical_tags(r.get(6)),
        priority: r.get(7),
        github_number: r.get(8),
        github_url: r.get(9),
        github_state: r.get(10),
        github_synced_at: r.get(11),
        created_at: r.get(12),
        updated_at: r.get(13),
        done_at: r.get(14),
        github_pr: r.get(15),
        content_changed_at: r.get(16),
        github_base: r.get::<_, Option<serde_json::Value>>(17).as_ref().and_then(IssueBase::from_json),
    }
}

/// Tags sous leur nom d'affichage : un tag enregistré sous le nom d'un label GitHub (`enhancement`, importé par
/// une ancienne version) est montré comme le tag intégré correspondant (« Amélioration »).
fn canonical_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for t in tags.iter().map(|t| label_to_tag(t)) {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// `[["bug", "d73a4a"], …]` → [(nom, couleur)]
pub(super) fn labels_from_json(v: &serde_json::Value) -> Vec<(String, String)> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|l| Some((l.get(0)?.as_str()?.to_string(), l.get(1)?.as_str().unwrap_or("").to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Labels du dépôt proposés comme tags : sous le nom du tag intégré quand il y en a un (`bug` → « Bug »),
/// comme les tags des tickets.
fn display_labels(labels: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for (name, color) in labels {
        let name = label_to_tag(&name);
        if !out.iter().any(|(n, _)| *n == name) {
            out.push((name, color));
        }
    }
    out
}

pub(super) async fn load_board(c: &Client) -> Res<Board> {
    let projects = c
        .query(
            "SELECT id, name, description, github_repo, github_labels FROM projects ORDER BY position, lower(name)",
            &[],
        )
        .await?
        .iter()
        .map(|r| Project {
            id: r.get(0),
            name: r.get(1),
            description: r.get(2),
            github_repo: r.get(3),
            github_labels: display_labels(labels_from_json(&r.get::<_, serde_json::Value>(4))),
        })
        .collect();
    let tickets = c
        .query(
            &format!(
                "SELECT {TICKET_COLS} FROM tickets t ORDER BY t.project_id, t.priority DESC, t.position, t.number"
            ),
            &[],
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_names_for_display() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(canonical_tags(v(&["enhancement", "Bug", "bug", "à moi"])), v(&["Amélioration", "Bug", "à moi"]));
        let labels = vec![("bug".to_string(), "d73a4a".to_string()), ("help wanted".to_string(), "008672".to_string())];
        assert_eq!(
            display_labels(labels),
            vec![("Bug".to_string(), "d73a4a".to_string()), ("help wanted".to_string(), "008672".to_string())]
        );
    }
}
