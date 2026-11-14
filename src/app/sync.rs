//! Lien avec la base : connexion, messages reçus du worker Postgres (Msg), événements live
//! (LISTEN/NOTIFY) et rechargements groupés (Needs).

use super::*;

impl MnemoApp {
    pub(super) fn connect(&mut self) {
        self.cfg.database_url = self.url_input.trim().to_string();
        self.cfg.save();
        if self.cfg.database_url.is_empty() {
            self.disconnect();
            return;
        }
        self.conn = ConnState::Connecting;
        self.offline_noticed = false;
        self.db.send(Cmd::Connect(self.cfg.database_url.clone()));
    }

    /// Arrête la synchronisation : Mnemo continue avec le dossier local.
    pub(super) fn disconnect(&mut self) {
        self.cfg.database_url.clear();
        self.url_input.clear();
        self.cfg.save();
        self.db.send(Cmd::Disconnect);
        self.conn = ConnState::Disconnected(None);
        self.live = false;
    }

    pub(super) fn handle_messages(&mut self, now: f64) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Connecting => self.conn = ConnState::Connecting,
                Msg::Connected => {
                    let was_offline = std::mem::replace(&mut self.offline_noticed, false);
                    self.conn = ConnState::Connected;
                    self.need.board = true;
                    self.send_github_config();
                    self.send_vault();
                    if was_offline {
                        self.notice(
                            "Base de nouveau joignable : synchronisation des changements faits hors ligne",
                            false,
                            now,
                        );
                    }
                }
                Msg::ConnectionFailed(e) => {
                    // une seule alerte par coupure (le worker réessaie toutes les 30 s)
                    if !std::mem::replace(&mut self.offline_noticed, true) {
                        self.notice(
                            format!("Synchronisation indisponible ({e}). Mnemo continue en local et réessaiera."),
                            true,
                            now,
                        );
                    }
                    self.conn = ConnState::Disconnected(Some(e));
                    self.live = false;
                }
                Msg::Disconnected => {
                    self.conn = ConnState::Disconnected(None);
                    self.live = false;
                }
                Msg::LocalReady => {
                    self.local_ready = true;
                    self.restore_done = false;
                    self.need.notes = true;
                    self.need.folders = true;
                    self.need.plugins = true;
                    self.need.tags = true;
                    self.need.icons = true;
                    self.need.files = true;
                }
                Msg::LocalEvent(ev) => self.handle_event(ev, now),
                Msg::Files(f) => {
                    self.files = f;
                    self.reload_whiteboards();
                    self.sketch_cache.clear();
                    self.rebuild_tree();
                    self.preview_gen = u64::MAX; // les images de l'aperçu sont à résoudre de nouveau
                    self.forget_images = true;
                }
                Msg::FileSaved { path, insert } => self.file_saved(path, insert, now),
                Msg::IdChanged { old, new } => self.remap_note_id(old, new),
                Msg::Live(v) => self.live = v,
                Msg::Resync => {
                    self.need = Needs {
                        notes: true,
                        folders: true,
                        plugins: true,
                        backlinks: true,
                        tags: true,
                        graph: self.graph_shown(),
                        board: true,
                        icons: true,
                        files: true,
                    };
                    // la base a pu changer pendant la coupure : comparaison immédiate avec le dossier
                    self.db.send(Cmd::VaultScan);
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
                            } else if self.notes.is_empty()
                                && self.fresh_vault
                                && self.cfg.database_url.trim().is_empty()
                            {
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
                Msg::Loaded { note, purpose: LoadPurpose::Open } => self.set_open(note, now),
                Msg::Created { note, open: true } => {
                    // nouvelle note : on peut écrire tout de suite (l'éditeur est dessiné plus loin dans cette frame)
                    let id = editor_id(note.id);
                    self.set_open(note, now);
                    self.focus_editor = Some(id);
                }
                Msg::Loaded { note, purpose: LoadPurpose::Refresh } => self.apply_refresh(note, now),
                Msg::Created { .. } => {}
                Msg::Saved(note) => {
                    if self.sent_save.as_ref().is_some_and(|s| s.0 == note.id) {
                        self.sent_save = None;
                    }
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
                    if !self.open.as_ref().is_some_and(|n| n.id == current.id) {
                        // enregistrement refusé alors que la note n'est plus ouverte : le texte n'est pas perdu
                        if let Some((_, path, content)) = self.sent_save.take_if(|s| s.0 == current.id)
                            && content != current.content
                        {
                            self.rescue_copy(&path, content);
                        }
                        continue;
                    }
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
                        self.cfg.last_note = Some(note.path.clone());
                    }
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
                Msg::Icons(i) => self.icons = i,
                Msg::Graph(g) => self.graph.set_data(g),
                Msg::Plugins(states) => {
                    self.plugins.apply_states(states);
                    self.plugin_changed();
                }
                // événements de la base (autres appareils) : notes, dossiers, icônes et plugins passent par le
                // dossier local (synchronisation immédiate) ; le tableau de bord est lu directement dans la base
                Msg::Event(ev) => {
                    let mine = ev.updated_by.as_deref().is_some_and(|u| u.starts_with(self.client_id.as_str()));
                    match ev.kind.as_str() {
                        "board" => self.handle_event(ev, now),
                        _ if !mine => self.db.send(Cmd::VaultScan),
                        _ => {}
                    }
                }
                Msg::Board(b) => {
                    crate::board::set_label_colors(&b.projects);
                    self.board = b;
                }
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
                Msg::GithubRepos(r) => self.dash.repos = Some(r),
                Msg::VaultSynced { stats, first } => self.on_vault_synced(stats, first, now),
                Msg::Error(e) => {
                    // un enregistrement qui a échoué (fichier verrouillé, disque plein) est réessayé plus tard,
                    // au lieu de rester « en cours » pour toujours
                    if let Some(n) = self.open.as_mut().filter(|n| n.saving.is_some()) {
                        n.saving = None;
                        n.last_edit = Some(now + 5.0);
                    }
                    self.notice(e, true, now);
                }
            }
        }
    }

    pub(super) fn handle_event(&mut self, ev: VaultEvent, now: f64) {
        self.plugins.on_event(&ev);
        match ev.kind.as_str() {
            "note" => {
                self.need.notes = true;
                self.need.board = true; // report paths
                self.need.backlinks = true;
                self.need.tags = true;
                self.need.graph |= self.graph_shown();
                if !self.search_q.trim().is_empty() {
                    self.search_due.get_or_insert(now + 0.3);
                }
                let mine = ev.updated_by.as_deref() == Some(self.client_id.as_str());
                // note supprimée (seule, avec son dossier, ou hors de Mnemo) : ni onglet ni historique ne la gardent
                if ev.op == "delete"
                    && let Some(id) = ev.note_id()
                    && !self.open.as_ref().is_some_and(|n| n.id == id)
                {
                    self.forget_note(id);
                }
                if let Some(n) = &self.open
                    && Some(n.id) == ev.note_id()
                {
                    if ev.op == "delete" {
                        if !mine {
                            let p = n.path.clone();
                            let id = n.id;
                            // texte pas encore enregistré : gardé dans une note à part
                            if n.dirty() {
                                let content = n.content.clone();
                                self.rescue_copy(&p, content);
                            }
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
            "icon" => self.need.icons = true,
            "file" => self.need.files = true,
            _ => {}
        }
    }

    pub(super) fn plugin_changed(&mut self) {
        self.plugin_cmds = self.plugins.commands();
        self.plugin_gen += 1;
        self.preview_gen = u64::MAX;
    }

    /// Une note a pris l'identifiant qu'elle a dans la base : onglets, historique et note ouverte suivent.
    pub(super) fn remap_note_id(&mut self, old: Uuid, new: Uuid) {
        let fix = |x: &mut Uuid| {
            if *x == old {
                *x = new;
            }
        };
        for t in &mut self.tabs {
            if let Some(n) = &mut t.note {
                fix(n);
            }
            t.back.iter_mut().for_each(fix);
            t.fwd.iter_mut().for_each(fix);
        }
        if let Some(n) = &mut self.open {
            fix(&mut n.id);
        }
        if let Some((id, _, _)) = &mut self.title_edit {
            fix(id);
        }
        self.need.notes = true;
        self.need.backlinks = true;
    }

    pub(super) fn flush_needs(&mut self) {
        if !self.local_ready {
            return;
        }
        let mut n = std::mem::take(&mut self.need);
        // le tableau de bord vit dans la base : seulement quand elle est connectée
        if self.conn != ConnState::Connected {
            self.need.board = n.board;
            n.board = false;
        }
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
        if n.icons {
            self.db.send(Cmd::ListIcons);
        }
        if n.files {
            self.db.send(Cmd::ListFiles);
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
}
