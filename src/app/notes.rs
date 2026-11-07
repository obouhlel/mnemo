//! Note ouverte et onglets : ouverture (remplacer / nouvel onglet / historique), navigation
//! ← →, fermeture, rafraîchissement depuis la base, détection des conflits et sauvegarde automatique.

use super::*;

pub(super) const AUTOSAVE_DELAY: f64 = 0.6;

impl MnemoApp {
    pub(super) fn set_open(&mut self, note: Note, now: f64) {
        self.flush_save();
        let mut mode = std::mem::replace(&mut self.open_mode, OpenMode::Replace);
        let t = &self.tabs[self.active_tab];
        if mode == OpenMode::Replace && (t.dashboard || t.graph) {
            mode = OpenMode::NewTab;
        }
        match mode {
            OpenMode::NewTab => {
                self.tabs.push(Tab { note: Some(note.id), ..Default::default() });
                self.active_tab = self.tabs.len() - 1;
            }
            OpenMode::Replace => {
                let t = &mut self.tabs[self.active_tab];
                if let Some(prev) = t.note
                    && prev != note.id
                {
                    t.back.push(prev);
                    t.fwd.clear();
                }
                t.note = Some(note.id);
            }
            OpenMode::History => self.tabs[self.active_tab].note = Some(note.id),
        }
        // reveal the note in the file explorer
        let mut acc = String::new();
        for seg in note.path.split('/').collect::<Vec<_>>().iter().rev().skip(1).rev() {
            if !acc.is_empty() {
                acc.push('/');
            }
            acc.push_str(seg);
            self.expanded.insert(acc.to_lowercase());
        }
        self.cfg.last_note = Some(note.path.clone());
        self.cfg.save();
        self.db.send(Cmd::Backlinks(note.id));
        self.backlinks.clear();
        self.autocomplete = None;
        self.open = Some(OpenNote {
            id: note.id,
            path: note.path.clone(),
            saved: note.content.clone(),
            content: note.content,
            version: note.version,
            last_edit: None,
            saving: None,
            conflict: None,
            remote_flash: 0.0,
        });
        self.content_gen += 1;
        let n = self.open.as_ref().unwrap();
        let (id, path, content) = (n.id.to_string(), n.path.clone(), n.content.clone());
        self.plugins.hook("on_open", Some(&NoteCtx { id, path: &path, content: &content }));
        let _ = now;
    }

    /// A newer version of the open note arrived from the database (another device / window / SQL).
    pub(super) fn apply_refresh(&mut self, note: Note, now: f64) {
        let Some(n) = self.open.as_mut().filter(|n| n.id == note.id) else { return };
        if note.version <= n.version {
            return;
        }
        n.path = note.path.clone();
        if !n.dirty() {
            n.content = note.content.clone();
            n.saved = note.content;
            n.version = note.version;
            n.remote_flash = now;
            self.content_gen += 1;
        } else if note.content != n.content {
            if n.saving.is_some() {
                return; // our pending save will be rejected and report the conflict itself
            }
            self.on_divergence(note, now);
        } else {
            n.version = note.version;
            n.saved = note.content;
        }
    }

    /// Local edits and a newer remote version diverged: merge automatically when the
    /// edits don't overlap, otherwise open the GitHub-like resolver.
    pub(super) fn on_divergence(&mut self, remote: Note, now: f64) {
        let Some(n) = self.open.as_mut().filter(|n| n.id == remote.id) else { return };
        let blocks = merge::build(&n.saved, &n.content, &remote.content);
        let (total, _) = merge::conflict_count(&blocks);
        if total == 0 {
            n.content = merge::assemble(&blocks).unwrap_or_else(|| n.content.clone());
            n.saved = remote.content;
            n.version = remote.version;
            n.conflict = None;
            n.remote_flash = now;
            n.last_edit = Some(now - AUTOSAVE_DELAY);
            self.content_gen += 1;
            self.notice("Modifications distantes fusionnées automatiquement avec les vôtres", false, now);
        } else {
            n.conflict = Some(Conflict {
                remote,
                blocks,
                tab: ConflictTab::Resolve,
                diff_against_result: false,
                window_open: true,
            });
            self.notice(format!("Conflit : {total} bloc(s) modifié(s) des deux côtés à résoudre"), true, now);
        }
    }

    pub(super) fn open_id(&mut self, id: Uuid) {
        self.open_in(id, OpenMode::Replace);
    }

    pub(super) fn open_in(&mut self, id: Uuid, mode: OpenMode) {
        if mode != OpenMode::NewTab && self.open.as_ref().is_some_and(|n| n.id == id) {
            if mode == OpenMode::History {
                self.tabs[self.active_tab].note = Some(id);
            }
            return;
        }
        self.flush_save();
        self.open_mode = mode;
        self.db.send(Cmd::Load { id, purpose: LoadPurpose::Open });
    }

    pub(super) fn activate_tab(&mut self, i: usize) {
        self.active_tab = i.min(self.tabs.len() - 1);
        if self.tabs[self.active_tab].dashboard {
            self.flush_save();
            self.open = None;
            self.need.board = true;
            return;
        }
        if self.tabs[self.active_tab].graph {
            self.flush_save();
            self.open = None;
            self.need.graph = true;
            return;
        }
        match self.tabs[self.active_tab].note {
            Some(id) => self.open_in(id, OpenMode::History),
            None => {
                self.flush_save();
                self.open = None;
            }
        }
    }

    pub(super) fn close_tab(&mut self, i: usize) {
        if i >= self.tabs.len() {
            return;
        }
        let was_active = i == self.active_tab;
        self.tabs.remove(i);
        if self.tabs.is_empty() {
            self.tabs.push(Tab::default());
        }
        if i < self.active_tab || self.active_tab >= self.tabs.len() {
            self.active_tab = self.active_tab.saturating_sub(1);
        }
        if was_active {
            self.activate_tab(self.active_tab);
        }
    }

    /// A note was deleted: drop it from tabs and histories.
    pub(super) fn forget_note(&mut self, id: Uuid) {
        for t in &mut self.tabs {
            t.back.retain(|x| *x != id);
            t.fwd.retain(|x| *x != id);
            if t.note == Some(id) {
                t.note = None;
            }
        }
    }

    pub(super) fn navigate(&mut self, forward: bool) {
        let t = &mut self.tabs[self.active_tab];
        let target = if forward { t.fwd.pop() } else { t.back.pop() };
        if let Some(target) = target {
            if let Some(cur) = t.note {
                if forward { t.back.push(cur) } else { t.fwd.push(cur) }
            }
            self.open_in(target, OpenMode::History);
        }
    }

    /// Un onglet « Graphe » est ouvert (il faut alors garder les données du graphe à jour).
    pub(super) fn graph_shown(&self) -> bool {
        self.tabs.iter().any(|t| t.graph)
    }

    /// Ouvre la vue graphe dans un onglet (réutilise l'onglet existant s'il y en a un).
    pub(super) fn open_graph(&mut self) {
        self.flush_save();
        match self.tabs.iter().position(|t| t.graph) {
            Some(i) => self.active_tab = i,
            None => {
                self.tabs.push(Tab { graph: true, ..Default::default() });
                self.active_tab = self.tabs.len() - 1;
            }
        }
        self.open = None;
        self.need.graph = true;
    }

    pub(super) fn tab_title(&self, t: &Tab) -> String {
        if t.dashboard {
            return "Tableau de bord".into();
        }
        if t.graph {
            return "Graphe".into();
        }
        match t.note {
            None => "Nouvel onglet".into(),
            Some(id) => self
                .notes
                .iter()
                .find(|n| n.id == id)
                .map(|n| basename(&n.path).to_string())
                .or_else(|| self.open.as_ref().filter(|o| o.id == id).map(|o| basename(&o.path).to_string()))
                .unwrap_or_else(|| "…".into()),
        }
    }

    pub(super) fn flush_save(&mut self) {
        if let Some(n) = &mut self.open
            && n.saving.is_none()
            && n.conflict.is_none()
            && n.content != n.saved
        {
            n.saving = Some(n.content.clone());
            n.last_edit = None;
            self.db.send(Cmd::Save { id: n.id, content: n.content.clone(), base_version: n.version });
        }
    }

    pub(super) fn tick_autosave(&mut self, ctx: &egui::Context, now: f64) {
        let Some(n) = &self.open else { return };
        if n.saving.is_some() || n.conflict.is_some() || n.content == n.saved {
            return;
        }
        if let Some(t) = n.last_edit {
            if now - t >= AUTOSAVE_DELAY {
                self.flush_save();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(AUTOSAVE_DELAY - (now - t) + 0.01));
            }
        }
    }
}
