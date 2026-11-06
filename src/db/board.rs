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
                    .execute(
                        "INSERT INTO projects (name, description, github_repo, position)
                         VALUES ($1, $2, $3, COALESCE((SELECT max(position) FROM projects), 0) + 1)",
                        &[&name.trim(), &description, &repo],
                    )
                    .await
                {
                    Ok(_) => t.commit().await?,
                    Err(e) if is_unique_violation(&e) => {
                        em.emit(Msg::Error(format!("Le projet « {name} » existe déjà")))
                    }
                    Err(e) => return Err(e),
                }
            }
            Cmd::UpdateProject { id, name, description, repo } => {
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                match t
                    .execute(
                        "UPDATE projects SET name = $2, description = $3, github_repo = $4 WHERE id = $1",
                        &[&id, &name.trim(), &description, &repo],
                    )
                    .await
                {
                    Ok(_) => t.commit().await?,
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
  t.github_state, t.github_synced_at, t.created_at, t.updated_at, t.done_at";

pub(super) const TICKET_COLS_N: usize = 15;

pub(super) fn ticket_from(r: &Row) -> Ticket {
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

pub(super) async fn load_board(c: &Client) -> Res<Board> {
    let projects = c
        .query("SELECT id, name, description, github_repo FROM projects ORDER BY position, lower(name)", &[])
        .await?
        .iter()
        .map(|r| Project { id: r.get(0), name: r.get(1), description: r.get(2), github_repo: r.get(3) })
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
