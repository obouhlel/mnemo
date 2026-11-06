//! Commandes sur les notes, les dossiers, la recherche, le graphe et l'état des plugins,
//! plus la réécriture des liens `[[...]]` lors d'un renommage.

use super::*;

impl Worker {
    /// Commandes notes / dossiers / recherche / plugins.
    pub(super) async fn note_cmd(&mut self, cmd: Cmd, em: &Emitter) -> Res<()> {
        match cmd {
            Cmd::ListNotes => {
                let rows = self
                    .conn()
                    .query("SELECT id, path, version, updated_at FROM notes ORDER BY lower(path)", &[])
                    .await?;
                let notes = rows
                    .iter()
                    .map(|r| NoteMeta { id: r.get(0), path: r.get(1), version: r.get(2), updated_at: r.get(3) })
                    .collect();
                em.emit(Msg::Notes(notes));
            }
            Cmd::ListFolders => {
                let rows = self.conn().query("SELECT path FROM folders ORDER BY lower(path)", &[]).await?;
                em.emit(Msg::Folders(rows.iter().map(|r| r.get(0)).collect()));
            }
            Cmd::Load { id, purpose } => {
                match self.conn().query_opt(&format!("SELECT {NOTE_COLS} FROM notes WHERE id = $1"), &[&id]).await? {
                    Some(r) => em.emit(Msg::Loaded { note: note_from(&r), purpose }),
                    None if purpose == LoadPurpose::Open => em.emit(Msg::Error("Note introuvable".into())),
                    None => {}
                }
            }
            Cmd::OpenLink(link) => {
                let link = link.trim().trim_end_matches(".md").to_string();
                if let Some(r) = self.conn().query_opt(RESOLVE_SQL, &[&link]).await? {
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
                    && let Some(r) = self
                        .conn()
                        .query_opt(&format!("SELECT {NOTE_COLS} FROM notes WHERE lower(path) = lower($1)"), &[&path])
                        .await?
                {
                    if open {
                        em.emit(Msg::Loaded { note: note_from(&r), purpose: LoadPurpose::Open });
                    }
                    return Ok(());
                }
                let t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                let r = t
                    .query_one(
                        &format!("INSERT INTO notes (path, content) VALUES ($1, $2) RETURNING {NOTE_COLS}"),
                        &[&path, &content],
                    )
                    .await;
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
                let cur =
                    t.query_opt(&format!("SELECT {NOTE_COLS} FROM notes WHERE id = $1 FOR UPDATE"), &[&id]).await?;
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
                let row = t
                    .query_opt(
                        &format!("UPDATE notes SET content = $2 WHERE id = $1 RETURNING {NOTE_COLS}"),
                        &[&id, &content],
                    )
                    .await?;
                t.commit().await?;
                em.emit(Msg::Saved(row.map(|r| note_from(&r)).unwrap_or(current)));
            }
            Cmd::Rename { id, new_path } => {
                let mut t = tagged_tx(self.client.as_mut().expect("connected"), &self.client_id).await?;
                let old: Option<String> =
                    t.query_opt("SELECT path FROM notes WHERE id = $1", &[&id]).await?.map(|r| r.get(0));
                let Some(old) = old else { return Ok(()) };
                if old == new_path {
                    return Ok(()); // nothing to do (e.g. the same rename sent twice)
                }
                let row = match t
                    .query_one(
                        &format!("UPDATE notes SET path = $2 WHERE id = $1 RETURNING {NOTE_COLS}"),
                        &[&id, &new_path],
                    )
                    .await
                {
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
                let folders =
                    t.query(&format!("DELETE FROM folders WHERE {PREFIXED} RETURNING path"), &[&from]).await?;
                for f in &folders {
                    let p: String = f.get(0);
                    t.execute(
                        "INSERT INTO folders (path) VALUES ($1) ON CONFLICT DO NOTHING",
                        &[&format!("{to}{}", &p[from.len()..])],
                    )
                    .await?;
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
                    .conn()
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
                let Some(row) = self.conn().query_opt("SELECT path FROM notes WHERE id = $1", &[&id]).await? else {
                    return Ok(());
                };
                let path: String = row.get(0);
                let base = basename(&path).to_string();
                let rows = self
                    .conn()
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
                                [&pl, &bl]
                                    .iter()
                                    .any(|t| ["]]", "|", "#"].iter().any(|end| l.contains(&format!("[[{t}{end}"))))
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
                    .conn()
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
                let nodes = self.conn().query("SELECT id::text, path FROM notes", &[]).await?;
                let edges = self
                    .conn()
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
                let mut g =
                    GraphData { nodes: nodes.iter().map(|r| (r.get(0), r.get(1), false)).collect(), edges: vec![] };
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
                let rows = self.conn().query("SELECT id, enabled, settings FROM plugin_state", &[]).await?;
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
            _ => unreachable!("commande routée ailleurs par Worker::handle"),
        }
        Ok(())
    }
}

pub(super) const NOTE_COLS: &str = "id, path, content, version, updated_by, updated_at";

pub(super) const PREFIXED: &str = "(lower(path) = lower($1) OR left(lower(path), length($1) + 1) = lower($1) || '/')";

pub(super) const RESOLVE_SQL: &str = "SELECT id, path, content, version, updated_by, updated_at FROM notes
  WHERE lower(path) = lower($1) OR lower(regexp_replace(path, '^.*/', '')) = lower($1)
  ORDER BY (lower(path) = lower($1)) DESC, length(path) LIMIT 1";

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

/// After a rename, rewrites `[[old]]`, `[[old|…]]`, `[[old#…]]` in every note linking to it.
pub(super) async fn relink(t: &mut tokio_postgres::Transaction<'_>, id: Uuid, old: &str, new: &str) -> Res<usize> {
    let old_base = basename(old).to_string();
    // Use the full path if the new basename becomes ambiguous
    let same: i64 = t
        .query_one(
            "SELECT count(*) FROM notes WHERE lower(regexp_replace(path, '^.*/', '')) = lower($1) AND id <> $2",
            &[&basename(new), &id],
        )
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

#[cfg(test)]
mod tests {
    use super::rewrite_links;

    #[test]
    fn rewrite() {
        let s = "a [[Old]] b [[old|alias]] c [[Dir/Old#h]] d [[Older]] [[x]]";
        assert_eq!(
            rewrite_links(s, &["Dir/Old", "Old"], "New"),
            "a [[New]] b [[New|alias]] c [[New#h]] d [[Older]] [[x]]"
        );
    }
}
