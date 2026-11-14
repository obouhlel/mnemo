//! Plugin system based on the Rhai scripting language (pure Rust, sandboxed, cross-platform).
//!
//! A plugin is a folder containing a `plugin.toml` manifest and a Rhai script. Every hook is
//! optional:
//!
//! ```rhai
//! fn on_load() {}                         // called when the plugin is enabled
//! fn commands() { [#{ id: "x", name: "Ma commande" }] }
//! fn run(command_id, note) {}             // note is () when no note is open
//! fn status(note) { "texte de la barre d'état" }
//! fn panel(note) { "## markdown affiché dans le panneau de droite" }
//! fn preprocess(markdown, note) { markdown }   // transforms the preview
//! fn on_open(note) {}
//! fn on_save(note) {}
//! fn on_event(event) {}                    // live events from the database
//! ```
//!
//! Host API: `notice(msg)`, `log(msg)`, `open_note(path)`, `create_note(path, content)`,
//! `insert_text(text)`, `replace_content(text)`, `setting(key)`, `note_paths()`, `date(fmt)`,
//! `date_offset(days, fmt)`, `word_count(text)`, `state_get(key)`, `state_set(key, value)`.
//!
//! Bac à sable (voir `docs/plugins.md`) : opérations, profondeur d'appel, tailles des chaînes / tableaux / maps
//! bornées, pas d'`import`, script principal dans le dossier du plugin, et une panique pendant un appel devient
//! une erreur du plugin.

use crate::model::PluginState;
use rhai::{AST, Array, Dynamic, Engine, EvalAltResult, FuncArgs, Map, Scope};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Deserialize)]
pub struct SettingDef {
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub default: Option<toml::Value>,
    /// "text" (default), "multiline" or "bool"
    #[serde(default)]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default = "default_main")]
    pub main: String,
    #[serde(default)]
    pub settings: Vec<SettingDef>,
}
fn default_main() -> String {
    "main.rhai".into()
}

pub struct Plugin {
    pub manifest: Manifest,
    #[allow(dead_code)]
    pub dir: PathBuf,
    pub enabled: bool,
    pub error: Option<String>,
    ast: Option<AST>,
    loaded: bool,
}

impl Plugin {
    fn has_fn(&self, name: &str, arity: usize) -> bool {
        self.ast.as_ref().is_some_and(|a| a.iter_functions().any(|f| f.name == name && f.params.len() == arity))
    }
    fn active(&self) -> bool {
        self.enabled && self.ast.is_some() && self.error.is_none()
    }
}

#[derive(Debug, Clone)]
pub enum Action {
    Notice(String),
    OpenNote(String),
    CreateNote { path: String, content: String },
    InsertText(String),
    ReplaceContent(String),
}

#[derive(Default)]
struct Shared {
    actions: Mutex<Vec<Action>>,
    current: Mutex<String>,
    settings: Mutex<HashMap<String, Map>>,
    state: Mutex<HashMap<String, Map>>,
    note_paths: Mutex<Vec<String>>,
    /// heure limite de l'appel en cours
    deadline: Mutex<Option<Instant>>,
}

/// Verrou qui survit à une panique survenue pendant un appel de plugin (mutex « empoisonné »).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Budget d'opérations d'un appel. Les hooks rappelés à chaque modification de la note (`status`, `panel`,
/// `preprocess`, `commands`) ont un budget plus serré que ceux déclenchés par une action de l'utilisateur.
const OPS_FREQUENT: u64 = 1_000_000;
const OPS_ACTION: u64 = 5_000_000;
/// Tailles maximales des valeurs d'un script (éléments imbriqués compris).
const MAX_STRING: usize = 10_000_000;
const MAX_ARRAY: usize = 100_000;
const MAX_MAP: usize = 10_000;
/// Actions (notifications, insertions…) gardées par appel, et clés de `state_set` par plugin.
const MAX_ACTIONS: usize = 200;
const MAX_STATE_KEYS: usize = 1_000;

/// Durée maximale d'un appel : certaines opérations coûtent bien plus que d'autres (grands tableaux), le
/// nombre d'opérations seul ne borne pas le temps.
const TIME_FREQUENT: Duration = Duration::from_secs(2);
const TIME_ACTION: Duration = Duration::from_secs(10);

fn budget(hook: &str) -> (u64, Duration) {
    if matches!(hook, "status" | "panel" | "preprocess" | "commands") {
        (OPS_FREQUENT, TIME_FREQUENT)
    } else {
        (OPS_ACTION, TIME_ACTION)
    }
}

/// Date formatée, ou une erreur Rhai si le format strftime est invalide (`%Q`, `100%`) : l'affichage d'un
/// format invalide par chrono panique.
fn format_date(date: chrono::DateTime<chrono::Local>, fmt: &str) -> Result<String, Box<EvalAltResult>> {
    let items: Vec<chrono::format::Item> = chrono::format::StrftimeItems::new(fmt).collect();
    if items.iter().any(|i| matches!(i, chrono::format::Item::Error)) {
        return Err(format!("format de date invalide : « {fmt} »").into());
    }
    let mut out = String::new();
    use std::fmt::Write;
    write!(out, "{}", date.format_with_items(items.iter()))
        .map_err(|_| format!("format de date invalide : « {fmt} »"))?;
    Ok(out)
}

/// Aujourd'hui décalé de `days` jours ; erreur si la date sort du calendrier.
fn offset_date(days: i64) -> Result<chrono::DateTime<chrono::Local>, Box<EvalAltResult>> {
    chrono::Duration::try_days(days)
        .and_then(|d| chrono::Local::now().checked_add_signed(d))
        .ok_or_else(|| format!("décalage de date hors limites : {days} jours").into())
}

/// `main` du manifeste : un simple nom de fichier du dossier du plugin (ni chemin absolu, ni `..`, ni sous-dossier).
fn valid_main(main: &str) -> bool {
    let mut parts = Path::new(main).components();
    matches!((parts.next(), parts.next()), (Some(std::path::Component::Normal(n)), None) if n == main)
        && !main.contains(['/', '\\', ':'])
}

/// Data about the current note handed to plugins.
pub struct NoteCtx<'a> {
    pub id: String,
    pub path: &'a str,
    pub content: &'a str,
}

fn note_dyn(note: Option<&NoteCtx>) -> Dynamic {
    match note {
        None => Dynamic::UNIT,
        Some(n) => {
            let mut m = Map::new();
            m.insert("id".into(), n.id.clone().into());
            m.insert("path".into(), n.path.to_string().into());
            m.insert("title".into(), crate::model::basename(n.path).to_string().into());
            m.insert("content".into(), n.content.to_string().into());
            m.into()
        }
    }
}

fn json_to_dyn(v: &serde_json::Value) -> Dynamic {
    match v {
        serde_json::Value::Bool(b) => (*b).into(),
        serde_json::Value::Number(n) => {
            n.as_i64().map(Dynamic::from).or_else(|| n.as_f64().map(Dynamic::from)).unwrap_or(Dynamic::UNIT)
        }
        serde_json::Value::String(s) => s.clone().into(),
        serde_json::Value::Null => Dynamic::UNIT,
        other => other.to_string().into(),
    }
}

pub fn toml_to_json(v: &toml::Value) -> serde_json::Value {
    match v {
        toml::Value::Boolean(b) => (*b).into(),
        toml::Value::Integer(i) => (*i).into(),
        toml::Value::Float(f) => (*f).into(),
        toml::Value::String(s) => s.clone().into(),
        other => other.to_string().into(),
    }
}

pub struct PluginHost {
    engine: Engine,
    shared: Arc<Shared>,
    pub plugins: Vec<Plugin>,
    pub dir: PathBuf,
    states: HashMap<String, PluginState>,
}

impl PluginHost {
    pub fn new(dir: PathBuf) -> Self {
        let shared = Arc::new(Shared::default());
        let mut engine = Engine::new();
        engine.set_max_operations(OPS_ACTION); // protects the UI from infinite loops (ajusté à chaque appel)
        engine.set_max_call_levels(64);
        engine.set_max_expr_depths(256, 128);
        engine.set_max_string_size(MAX_STRING);
        engine.set_max_array_size(MAX_ARRAY);
        engine.set_max_map_size(MAX_MAP);
        // pas d'`import` : le résolveur par défaut lirait n'importe quel fichier .rhai du disque
        engine.set_module_resolver(rhai::module_resolvers::DummyModuleResolver::new());

        let s = shared.clone();
        engine.on_progress(move |ops| {
            let late = ops % 512 == 0 && lock(&s.deadline).is_some_and(|d| Instant::now() > d);
            late.then(|| "temps d'exécution dépassé".into())
        });

        let push = |s: &Arc<Shared>| {
            let s = s.clone();
            move |a: Action| {
                let mut actions = lock(&s.actions);
                if actions.len() < MAX_ACTIONS {
                    actions.push(a);
                }
            }
        };
        let p = push(&shared);
        engine.register_fn("notice", move |msg: &str| p(Action::Notice(msg.to_string())));
        let p = push(&shared);
        engine.register_fn("open_note", move |path: &str| p(Action::OpenNote(path.to_string())));
        let p = push(&shared);
        engine.register_fn("create_note", move |path: &str, content: &str| {
            p(Action::CreateNote { path: path.to_string(), content: content.to_string() })
        });
        let p = push(&shared);
        engine.register_fn("insert_text", move |t: &str| p(Action::InsertText(t.to_string())));
        let p = push(&shared);
        engine.register_fn("replace_content", move |t: &str| p(Action::ReplaceContent(t.to_string())));

        let s = shared.clone();
        engine.register_fn("log", move |msg: &str| {
            eprintln!("[plugin:{}] {msg}", lock(&s.current));
        });
        let s = shared.clone();
        engine.register_fn("setting", move |key: &str| -> Dynamic {
            let cur = lock(&s.current).clone();
            lock(&s.settings).get(&cur).and_then(|m| m.get(key).cloned()).unwrap_or(Dynamic::UNIT)
        });
        let s = shared.clone();
        engine.register_fn("state_get", move |key: &str| -> Dynamic {
            let cur = lock(&s.current).clone();
            lock(&s.state).get(&cur).and_then(|m| m.get(key).cloned()).unwrap_or(Dynamic::UNIT)
        });
        let s = shared.clone();
        engine.register_fn("state_set", move |key: &str, value: Dynamic| -> Result<(), Box<EvalAltResult>> {
            let cur = lock(&s.current).clone();
            let mut state = lock(&s.state);
            let map = state.entry(cur).or_default();
            if map.len() >= MAX_STATE_KEYS && !map.contains_key(key) {
                return Err(format!("state_set : plus de {MAX_STATE_KEYS} clés").into());
            }
            map.insert(key.into(), value);
            Ok(())
        });
        let s = shared.clone();
        engine.register_fn("note_paths", move || -> Array {
            lock(&s.note_paths).iter().map(|p| Dynamic::from(p.clone())).collect()
        });
        engine.register_fn("date", |fmt: &str| format_date(chrono::Local::now(), fmt));
        engine.register_fn("date_offset", |days: i64, fmt: &str| format_date(offset_date(days)?, fmt));
        engine.register_fn("word_count", |t: &str| {
            t.split_whitespace().filter(|w| w.chars().any(char::is_alphanumeric)).count() as i64
        });
        engine.register_fn("char_count", |t: &str| t.chars().count() as i64);

        Self { engine, shared, plugins: vec![], dir, states: HashMap::new() }
    }

    /// Installs the bundled example plugins when the plugins folder does not exist yet.
    pub fn install_bundled(&self, overwrite: bool) -> std::io::Result<()> {
        for (id, files) in BUNDLED {
            let d = self.dir.join(id);
            if d.exists() && !overwrite {
                continue;
            }
            std::fs::create_dir_all(&d)?;
            for (name, content) in *files {
                std::fs::write(d.join(name), content)?;
            }
        }
        Ok(())
    }

    /// (Re)scans the plugins folder and compiles every plugin.
    pub fn reload(&mut self) {
        if !self.dir.exists()
            && let Err(e) = self.install_bundled(false)
        {
            eprintln!("[plugins] cannot install bundled plugins: {e}");
        }
        self.plugins.clear();
        let mut dirs: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.join("plugin.toml").is_file()).collect())
            .unwrap_or_default();
        dirs.sort();
        for dir in dirs {
            match load_manifest(&dir) {
                Ok(manifest) => {
                    let (ast, error) = if !valid_main(&manifest.main) {
                        let e =
                            format!("main = « {} » : attendu un nom de fichier du dossier du plugin", manifest.main);
                        (None, Some(e))
                    } else {
                        match self.engine.compile_file(dir.join(&manifest.main)) {
                            Ok(a) => (Some(a), None),
                            Err(e) => (None, Some(format!("Erreur de compilation : {e}"))),
                        }
                    };
                    self.plugins.push(Plugin { manifest, dir, enabled: false, error, ast, loaded: false });
                }
                Err(e) => eprintln!("[plugins] {}: {e}", dir.display()),
            }
        }
        let states = std::mem::take(&mut self.states);
        self.apply_states(states);
    }

    /// Applies enabled flags and settings coming from the database (synced across machines).
    pub fn apply_states(&mut self, states: HashMap<String, PluginState>) {
        self.states = states;
        let mut to_load = vec![];
        for (i, p) in self.plugins.iter_mut().enumerate() {
            let st = self.states.get(&p.manifest.id);
            p.enabled = st.is_some_and(|s| s.enabled);
            // settings = manifest defaults overridden by stored values
            let mut m = Map::new();
            for def in &p.manifest.settings {
                m.insert(
                    def.key.as_str().into(),
                    def.default.as_ref().map(|d| json_to_dyn(&toml_to_json(d))).unwrap_or_else(|| "".into()),
                );
            }
            if let Some(st) = st {
                for (k, v) in &st.settings {
                    m.insert(k.as_str().into(), json_to_dyn(v));
                }
            }
            lock(&self.shared.settings).insert(p.manifest.id.clone(), m);
            if p.enabled && !p.loaded {
                to_load.push(i);
            }
            if !p.enabled {
                p.loaded = false;
            }
        }
        for i in to_load {
            self.plugins[i].loaded = true;
            if self.plugins[i].has_fn("on_load", 0) {
                let _: Option<Dynamic> = self.call(i, "on_load", ());
            }
        }
    }

    pub fn state_of(&self, id: &str) -> PluginState {
        self.states.get(id).cloned().unwrap_or_default()
    }

    pub fn set_note_paths(&self, paths: Vec<String>) {
        *lock(&self.shared.note_paths) = paths;
    }

    pub fn take_actions(&self) -> Vec<Action> {
        std::mem::take(&mut *lock(&self.shared.actions))
    }

    fn call<T: Clone + Send + Sync + 'static>(&mut self, i: usize, name: &str, args: impl FuncArgs) -> Option<T> {
        let p = &self.plugins[i];
        let ast = p.ast.as_ref()?;
        *lock(&self.shared.current) = p.manifest.id.clone();
        let (ops, time) = budget(name);
        self.engine.set_max_operations(ops);
        *lock(&self.shared.deadline) = Some(Instant::now() + time);
        let engine = &self.engine;
        // une panique dans le moteur ou dans une fonction de l'hôte ne doit pas emporter l'application
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut scope = Scope::new();
            let opts = rhai::CallFnOptions::new().eval_ast(false).rewind_scope(true);
            engine.call_fn_with_options::<Dynamic>(opts, &mut scope, ast, name, args).map_err(|e| e.to_string())
        }))
        .unwrap_or_else(|panic| {
            let what =
                panic.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| panic.downcast_ref::<String>().cloned());
            Err(format!("erreur interne ({})", what.unwrap_or_else(|| "panique".into())))
        });
        match result {
            Ok(v) => v.try_cast::<T>(),
            Err(e) => {
                let msg = format!("{name}() : {e}");
                eprintln!("[plugin:{}] {msg}", p.manifest.id);
                // hors de `push` : l'erreur s'affiche même si le plugin a rempli la file d'actions
                lock(&self.shared.actions).push(Action::Notice(format!("Plugin « {} » : {msg}", p.manifest.name)));
                self.plugins[i].error = Some(msg);
                None
            }
        }
    }

    fn each_with(&self, name: &str, arity: usize) -> Vec<usize> {
        (0..self.plugins.len()).filter(|&i| self.plugins[i].active() && self.plugins[i].has_fn(name, arity)).collect()
    }

    /// (plugin index, command id, label)
    pub fn commands(&mut self) -> Vec<(usize, String, String)> {
        let mut out = vec![];
        for i in self.each_with("commands", 0) {
            let Some(arr) = self.call::<Array>(i, "commands", ()) else { continue };
            for c in arr {
                if let Some(m) = c.try_cast::<Map>() {
                    let id = m.get("id").map(|v| v.to_string()).unwrap_or_default();
                    let name = m.get("name").map(|v| v.to_string()).unwrap_or_else(|| id.clone());
                    out.push((i, id, format!("{} : {}", self.plugins[i].manifest.name, name)));
                }
            }
        }
        out
    }

    pub fn run_command(&mut self, i: usize, id: &str, note: Option<&NoteCtx>) {
        if i < self.plugins.len() && self.plugins[i].active() && self.plugins[i].has_fn("run", 2) {
            let _: Option<Dynamic> = self.call(i, "run", (id.to_string(), note_dyn(note)));
        }
    }

    pub fn status(&mut self, note: Option<&NoteCtx>) -> Vec<String> {
        let nd = note_dyn(note);
        self.each_with("status", 1)
            .into_iter()
            .filter_map(|i| self.call::<Dynamic>(i, "status", (nd.clone(),)))
            .filter(|d| !d.is_unit())
            .map(|d| d.to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// (plugin name, markdown)
    pub fn panels(&mut self, note: Option<&NoteCtx>) -> Vec<(String, String)> {
        let nd = note_dyn(note);
        self.each_with("panel", 1)
            .into_iter()
            .filter_map(|i| {
                let d = self.call::<Dynamic>(i, "panel", (nd.clone(),))?;
                (!d.is_unit()).then(|| (self.plugins[i].manifest.name.clone(), d.to_string()))
            })
            .collect()
    }

    pub fn preprocess(&mut self, markdown: String, note: Option<&NoteCtx>) -> String {
        let nd = note_dyn(note);
        let mut md = markdown;
        for i in self.each_with("preprocess", 2) {
            if let Some(s) = self.call::<rhai::ImmutableString>(i, "preprocess", (md.clone(), nd.clone())) {
                md = s.to_string();
            }
        }
        md
    }

    pub fn hook(&mut self, name: &str, note: Option<&NoteCtx>) {
        let nd = note_dyn(note);
        for i in self.each_with(name, 1) {
            let _: Option<Dynamic> = self.call(i, name, (nd.clone(),));
        }
    }

    pub fn on_event(&mut self, ev: &crate::model::VaultEvent) {
        let mut m = Map::new();
        m.insert("kind".into(), ev.kind.clone().into());
        m.insert("op".into(), ev.op.clone().into());
        m.insert("path".into(), ev.path.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT));
        m.insert("remote".into(), true.into());
        let d: Dynamic = m.into();
        for i in self.each_with("on_event", 1) {
            let _: Option<Dynamic> = self.call(i, "on_event", (d.clone(),));
        }
    }

    pub fn clear_error(&mut self, i: usize) {
        if let Some(p) = self.plugins.get_mut(i) {
            p.error = None;
            p.loaded = false;
        }
        let st = std::mem::take(&mut self.states);
        self.apply_states(st);
    }
}

fn load_manifest(dir: &Path) -> Result<Manifest, String> {
    let s = std::fs::read_to_string(dir.join("plugin.toml")).map_err(|e| e.to_string())?;
    let m: Manifest = toml::from_str(&s).map_err(|e| e.to_string())?;
    if m.id.trim().is_empty() {
        return Err("id manquant".into());
    }
    Ok(m)
}

type Bundle = &'static [(&'static str, &'static [(&'static str, &'static str)])];
const BUNDLED: Bundle = &[
    (
        "word-count",
        &[
            ("plugin.toml", include_str!("../assets/plugins/word-count/plugin.toml")),
            ("main.rhai", include_str!("../assets/plugins/word-count/main.rhai")),
        ],
    ),
    (
        "daily-notes",
        &[
            ("plugin.toml", include_str!("../assets/plugins/daily-notes/plugin.toml")),
            ("main.rhai", include_str!("../assets/plugins/daily-notes/main.rhai")),
        ],
    ),
    (
        "outline",
        &[
            ("plugin.toml", include_str!("../assets/plugins/outline/plugin.toml")),
            ("main.rhai", include_str!("../assets/plugins/outline/main.rhai")),
        ],
    ),
    (
        "callouts",
        &[
            ("plugin.toml", include_str!("../assets/plugins/callouts/plugin.toml")),
            ("main.rhai", include_str!("../assets/plugins/callouts/main.rhai")),
        ],
    ),
    (
        "templates",
        &[
            ("plugin.toml", include_str!("../assets/plugins/templates/plugin.toml")),
            ("main.rhai", include_str!("../assets/plugins/templates/main.rhai")),
        ],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> PluginHost {
        let dir = std::env::temp_dir().join(format!("mnemo-plugins-test-{}", uuid::Uuid::new_v4()));
        let mut h = PluginHost::new(dir);
        h.reload();
        let states = h
            .plugins
            .iter()
            .map(|p| (p.manifest.id.clone(), PluginState { enabled: true, settings: Default::default() }))
            .collect();
        h.apply_states(states);
        h
    }

    #[test]
    fn bundled_plugins_work() {
        let mut h = host();
        assert_eq!(h.plugins.len(), BUNDLED.len());
        for p in &h.plugins {
            assert!(p.error.is_none(), "{}: {:?}", p.manifest.id, p.error);
        }
        let content = "# Titre\n\nUn deux trois.\n\n## Sous-partie\n\n> [!warning] Attention\n> texte\n";
        let note = NoteCtx { id: "x".into(), path: "Dossier/Ma note", content };
        let status = h.status(Some(&note));
        assert!(status.iter().any(|s| s.contains("mots")), "{status:?}");
        let panels = h.panels(Some(&note));
        assert!(panels.iter().any(|(_, md)| md.contains("Sous-partie")), "{panels:?}");
        let pre = h.preprocess(content.to_string(), Some(&note));
        assert!(!pre.contains("[!warning]"), "{pre}");

        let cmds = h.commands();
        assert!(cmds.len() >= 3, "{cmds:?}");
        let (i, id, _) = cmds.iter().find(|c| c.1 == "today").unwrap().clone();
        h.run_command(i, &id, None);
        let actions = h.take_actions();
        assert!(
            matches!(&actions[..], [Action::CreateNote { path, .. }] if path.starts_with("Journal/")),
            "{actions:?}"
        );
        for p in &h.plugins {
            assert!(p.error.is_none(), "{}: {:?}", p.manifest.id, p.error);
        }
    }

    /// Hôte avec un seul plugin `t` dont le script est `script`.
    fn host_with(script: &str, manifest_extra: &str) -> PluginHost {
        let dir = std::env::temp_dir().join(format!("mnemo-plugins-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("t")).unwrap();
        std::fs::write(dir.join("t/plugin.toml"), format!("id = \"t\"\nname = \"T\"\n{manifest_extra}")).unwrap();
        std::fs::write(dir.join("t/main.rhai"), script).unwrap();
        let mut h = PluginHost::new(dir);
        h.reload();
        h.apply_states([("t".to_string(), PluginState { enabled: true, settings: Default::default() })].into());
        h
    }

    fn status_of(script: &str) -> (Vec<String>, Option<String>) {
        let mut h = host_with(script, "");
        let out = h.status(None);
        let err = h.plugins[0].error.clone();
        let _ = std::fs::remove_dir_all(&h.dir);
        (out, err)
    }

    #[test]
    fn bundled_plugins_handle_a_large_note() {
        let mut h = host();
        let mut content = String::new();
        for i in 0..4000 {
            content.push_str(&format!(
                "## Titre {i}\n\nUn paragraphe de quelques mots, ligne {i}.\n\n> [!note] Note {i}\n> texte\n\n"
            ));
        }
        assert!(content.len() > 300_000);
        let note = NoteCtx { id: "x".into(), path: "Grande", content: &content };
        assert!(!h.status(Some(&note)).is_empty());
        assert!(!h.panels(Some(&note)).is_empty());
        assert!(!h.preprocess(content.clone(), Some(&note)).contains("[!note]"));
        for p in &h.plugins {
            assert!(p.error.is_none(), "{}: {:?}", p.manifest.id, p.error);
        }
    }

    #[test]
    fn bad_dates_are_plugin_errors() {
        for script in [
            r#"fn status(n) { date("%Y-%Q") }"#,
            r#"fn status(n) { date("100%") }"#,
            r#"fn status(n) { date_offset(9_000_000_000_000_000, "%Y") }"#,
            r#"fn status(n) { date_offset(-400_000_000, "%Y") }"#,
        ] {
            let (out, err) = status_of(script);
            assert!(out.is_empty() && err.is_some(), "{script}: {out:?} {err:?}");
        }
        let (out, err) = status_of(r#"fn status(n) { date("%d/%m/%Y %H:%M 100%%") + date_offset(-1, "%F") }"#);
        assert!(err.is_none() && out[0].contains("100%"), "{out:?} {err:?}");
    }

    #[test]
    fn sandbox() {
        // mémoire : tableaux, maps et chaînes bornés
        for script in [
            "fn status(n) { let a = []; loop { a.push(1); } }",
            "fn status(n) { let a = [0]; loop { a += a; } }",
            "fn status(n) { let m = #{}; let i = 0; loop { m[`k${i}`] = i; i += 1; } }",
            r#"fn status(n) { let s = "xxxxxxxxxxxxxxxx"; loop { s += s; } }"#,
            "fn status(n) { loop { } }",
        ] {
            let (out, err) = status_of(script);
            assert!(out.is_empty() && err.is_some(), "{script}: {out:?} {err:?}");
        }
        // pas d'import, même d'un fichier voisin
        let mut h = host_with("import \"autre\" as a;\nfn status(n) { a::x() }", "");
        std::fs::write(h.dir.join("t/autre.rhai"), "fn x() { 1 }").unwrap();
        h.reload();
        assert!(h.status(None).is_empty());
        assert!(h.plugins[0].error.is_some());
        let _ = std::fs::remove_dir_all(&h.dir);
        // main hors du dossier du plugin
        for main in ["../autre.rhai", "sous/main.rhai", "/etc/passwd", "..", "", "C:\\\\x.rhai"] {
            assert!(!valid_main(main), "{main}");
            let h = host_with("fn status(n) { 1 }", &format!("main = '{main}'\n"));
            assert!(h.plugins[0].error.as_deref().is_some_and(|e| e.contains("main")), "{main}");
            let _ = std::fs::remove_dir_all(&h.dir);
        }
        assert!(valid_main("main.rhai") && valid_main("mon script.rhai"));
        // file d'actions et état bornés
        let mut h = host_with(
            "fn run(id, n) { for i in 0..5000 { notice(`n${i}`); } }\nfn on_open(n) { for i in 0..5000 { state_set(`k${i}`, i); } }",
            "",
        );
        h.run_command(0, "x", None);
        assert_eq!(h.take_actions().len(), MAX_ACTIONS);
        h.hook("on_open", None);
        assert!(h.plugins[0].error.as_deref().is_some_and(|e| e.contains("state_set")));
        let _ = std::fs::remove_dir_all(&h.dir);
    }

    #[test]
    fn a_panic_in_a_call_is_a_plugin_error() {
        let mut h = host_with("fn status(n) { boom() }", "");
        h.engine.register_fn("boom", || -> i64 { panic!("aïe") });
        assert!(h.status(None).is_empty());
        assert!(h.plugins[0].error.as_deref().is_some_and(|e| e.contains("aïe")), "{:?}", h.plugins[0].error);
        // l'hôte reste utilisable (verrous compris)
        assert!(matches!(&h.take_actions()[..], [Action::Notice(m)] if m.contains("erreur interne")));
        let _ = std::fs::remove_dir_all(&h.dir);
    }
}
