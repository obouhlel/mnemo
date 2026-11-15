//! Signets (plugin cœur « Signets » d'Obsidian) : `.obsidian/bookmarks.json` du coffre, lu et écrit au même
//! format pour que les deux applications partagent la même liste ; `.mnemo/bookmarks.json` hors d'un coffre.
//! Ce fichier d'état reste local, comme dans Obsidian (il n'est pas synchronisé par la base).
//! Reprend aussi les plugins cœur activés dans Obsidian (`core-plugins.json`) à l'ouverture d'un coffre.

use super::*;
use serde_json::{Value, json};

/// Plugins cœur d'Obsidian qui ont un équivalent Rhai livré avec Mnemo (même identifiant).
const CORE_EQUIVALENTS: &[&str] = &["outline", "word-count", "daily-notes", "templates"];

/// Plugins cœur activés d'après `core-plugins.json` : objet `{ "outline": true, … }` (Obsidian ≥ 1.0) ou
/// liste des identifiants activés (anciennes versions).
pub(super) fn enabled_core_plugins(json: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(m)) => m.into_iter().filter(|(_, v)| v.as_bool() == Some(true)).map(|(k, _)| k).collect(),
        Ok(Value::Array(a)) => a.into_iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => vec![],
    }
}

/// Signet affiché : profondeur (groupes imbriqués), type, libellé et cible.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Bookmark {
    pub depth: usize,
    pub kind: String,
    pub title: String,
    /// chemin (fichier, dossier, titre), requête (recherche) ou adresse (url)
    pub target: String,
}

/// Liste aplatie des signets d'un `bookmarks.json`.
pub(super) fn parse_bookmarks(v: &Value) -> Vec<Bookmark> {
    fn walk(items: &[Value], depth: usize, out: &mut Vec<Bookmark>) {
        for it in items {
            let kind = it.get("type").and_then(Value::as_str).unwrap_or("").to_string();
            let s = |k: &str| it.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let target = match kind.as_str() {
                "search" => s("query"),
                "url" => s("url"),
                "heading" | "block" => format!("{}{}", s("path"), s("subpath")),
                _ => s("path"),
            };
            let title = match s("title") {
                t if !t.is_empty() => t,
                _ if kind == "file" || kind == "folder" => {
                    let p = s("path");
                    basename(p.trim_end_matches(".md")).to_string()
                }
                _ => target.clone(),
            };
            out.push(Bookmark { depth, kind: kind.clone(), title, target });
            if let Some(children) = it.get("items").and_then(Value::as_array) {
                walk(children, depth + 1, out);
            }
        }
    }
    let mut out = vec![];
    if let Some(items) = v.get("items").and_then(Value::as_array) {
        walk(items, 0, &mut out);
    }
    out
}

/// Vrai si la note `path` (sans `.md`) a un signet, à n'importe quel niveau.
fn has_file(v: &Value, path: &str) -> bool {
    parse_bookmarks(v).iter().any(|b| b.kind == "file" && b.target.eq_ignore_ascii_case(&format!("{path}.md")))
}

/// Ajoute ou retire (à tous les niveaux) le signet de la note `path` ; les autres champs sont gardés.
pub(super) fn toggle_file(v: &mut Value, path: &str, now_ms: i64) -> bool {
    let file = format!("{path}.md");
    if has_file(v, path) {
        fn remove(items: &mut Vec<Value>, file: &str) {
            items.retain(|it| {
                !(it.get("type").and_then(Value::as_str) == Some("file")
                    && it.get("path").and_then(Value::as_str).is_some_and(|p| p.eq_ignore_ascii_case(file)))
            });
            for it in items.iter_mut() {
                if let Some(children) = it.get_mut("items").and_then(Value::as_array_mut) {
                    remove(children, file);
                }
            }
        }
        if let Some(items) = v.get_mut("items").and_then(Value::as_array_mut) {
            remove(items, &file);
        }
        false
    } else {
        if !v.is_object() {
            *v = json!({});
        }
        let items = v.as_object_mut().unwrap().entry("items").or_insert_with(|| json!([]));
        if !items.is_array() {
            *items = json!([]);
        }
        items.as_array_mut().unwrap().push(json!({ "type": "file", "ctime": now_ms, "path": file }));
        true
    }
}

impl MnemoApp {
    /// Fichier des signets : celui du coffre Obsidian, sinon `.mnemo/bookmarks.json` du dossier des notes.
    pub(super) fn bookmarks_file(&self) -> Option<std::path::PathBuf> {
        match self.iconize_dir() {
            Some(obs) => Some(obs.join("bookmarks.json")),
            None => self.cfg.vault_dir.as_ref().map(|d| d.join(".mnemo").join("bookmarks.json")),
        }
    }

    /// Contenu du fichier des signets, relu seulement s'il a changé sur le disque.
    pub(super) fn bookmarks_json(&mut self) -> Value {
        let Some(file) = self.bookmarks_file() else { return json!({}) };
        let mtime = std::fs::metadata(&file).and_then(|m| m.modified()).ok();
        if self.bookmarks.as_ref().is_none_or(|(f, m, _)| *f != file || *m != mtime) {
            let v = std::fs::read_to_string(&file)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .unwrap_or_else(|| json!({ "items": [] }));
            self.bookmarks = Some((file, mtime, v));
        }
        self.bookmarks.as_ref().map(|b| b.2.clone()).unwrap_or_default()
    }

    /// Ajoute aux signets la note ouverte, ou l'en retire.
    pub(super) fn toggle_bookmark(&mut self, now: f64) {
        let Some(path) = self.open.as_ref().map(|n| n.path.clone()) else { return };
        let Some(file) = self.bookmarks_file() else {
            self.notice("Choisissez d'abord un dossier de notes", true, now);
            return;
        };
        let mut v = self.bookmarks_json();
        let added = toggle_file(&mut v, &path, chrono::Utc::now().timestamp_millis());
        let text = serde_json::to_string_pretty(&v).unwrap_or_default();
        let written = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&file, text));
        match written {
            Ok(()) => {
                self.bookmarks = None;
                let msg = if added { "Ajoutée aux signets" } else { "Retirée des signets" };
                self.notice(format!("« {} » : {}", basename(&path), msg.to_lowercase()), false, now);
            }
            Err(e) => self.notice(format!("Signets non enregistrés : {e}"), true, now),
        }
    }

    /// Vrai si la note ouverte a un signet.
    pub(super) fn open_is_bookmarked(&mut self) -> bool {
        let Some(path) = self.open.as_ref().map(|n| n.path.clone()) else { return false };
        has_file(&self.bookmarks_json(), &path)
    }

    /// Onglet « Signets » du panneau de gauche.
    pub(super) fn bookmarks_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, now: f64) {
        let p = self.palette();
        let list = parse_bookmarks(&self.bookmarks_json());
        if list.is_empty() {
            ui.add(
                egui::Label::new(
                    RichText::new("Aucun signet. Palette (Ctrl+P) → « Ajouter la note aux signets ».").color(p.faint),
                )
                .wrap(),
            );
        }
        for b in list {
            let icon = match b.kind.as_str() {
                "folder" => "folder",
                "search" => "search",
                "group" => "folder-open",
                "url" => "globe",
                "heading" | "block" => "heading",
                _ => "file-text",
            };
            let r = theme::tree_row_icon(ui, &b.title, b.depth, false, None, Some(icon)).on_hover_text(&b.target);
            if r.clicked() {
                match b.kind.as_str() {
                    "file" | "heading" | "block" => {
                        let path = b.target.split('#').next().unwrap_or("").trim_end_matches(".md").to_string();
                        if crate::markdown::is_attachment(&path) {
                            self.actions.push(UiAction::OpenFile(path));
                        } else {
                            self.actions.push(UiAction::OpenLink(path));
                        }
                    }
                    "folder" => self.show_pane(Pane::Files),
                    "search" => {
                        self.show_pane(Pane::Search);
                        self.search_q = b.target.clone();
                        self.search_due = Some(now);
                    }
                    "url" => ctx.open_url(egui::OpenUrl::new_tab(&b.target)),
                    _ => {}
                }
            }
        }
    }

    /// À l'ouverture d'un coffre Obsidian, les plugins cœur qu'il active (Plan, Compteur de mots, Notes
    /// quotidiennes, Modèles) activent leur équivalent Mnemo, sauf s'il a déjà été réglé ici.
    pub(super) fn adopt_core_plugins(&mut self, states: &std::collections::HashMap<String, PluginState>) {
        let Some(dir) = self.iconize_dir() else { return };
        if self.core_plugins_checked.as_ref() == Some(&dir) {
            return;
        }
        self.core_plugins_checked = Some(dir.clone());
        let Ok(json) = std::fs::read_to_string(dir.join("core-plugins.json")) else { return };
        for id in enabled_core_plugins(&json) {
            if CORE_EQUIVALENTS.contains(&id.as_str())
                && !states.contains_key(&id)
                && self.plugins.plugins.iter().any(|p| p.manifest.id == id)
            {
                let state = PluginState { enabled: true, settings: Default::default() };
                self.db.send(Cmd::SetPlugin { id, state });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn core_plugins() {
        assert_eq!(
            enabled_core_plugins(r#"{"outline": true, "graph": true, "word-count": false}"#),
            ["graph", "outline"]
        );
        assert_eq!(enabled_core_plugins(r#"["outline", "word-count"]"#), ["outline", "word-count"]);
        assert!(enabled_core_plugins("pas du json").is_empty());
    }

    #[test]
    fn bookmarks_roundtrip() {
        let mut v: Value = serde_json::from_str(
            r#"{"items":[{"type":"file","ctime":1,"path":"A/Note.md"},
                {"type":"group","ctime":2,"title":"Projets","items":[{"type":"file","ctime":3,"path":"B.md","title":"Bé"}]},
                {"type":"search","ctime":4,"query":"tag:#rust"},{"type":"folder","ctime":5,"path":"Assets"}]}"#,
        )
        .unwrap();
        let list = parse_bookmarks(&v);
        let titles: Vec<(usize, &str)> = list.iter().map(|b| (b.depth, b.title.as_str())).collect();
        assert_eq!(titles, [(0, "Note"), (0, "Projets"), (1, "Bé"), (0, "tag:#rust"), (0, "Assets")]);
        assert!(has_file(&v, "B"));
        // retrait dans un groupe, ajout à la racine
        assert!(!toggle_file(&mut v, "b", 9));
        assert!(!has_file(&v, "B"));
        assert!(toggle_file(&mut v, "C", 9));
        assert_eq!(
            v["items"].as_array().unwrap().last().unwrap(),
            &json!({"type": "file", "ctime": 9, "path": "C.md"})
        );
        let mut empty = json!({});
        assert!(toggle_file(&mut empty, "X", 1));
        assert_eq!(parse_bookmarks(&empty).len(), 1);
    }
}
