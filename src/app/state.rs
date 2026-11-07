//! Types qui décrivent l'état de l'interface : connexion, onglets, note ouverte,
//! dialogues, conflits, actions de l'interface, arborescence et thèmes CSS chargés.

use super::CoreCmd;
use crate::merge::Block;
use crate::model::Note;
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(PartialEq)]
pub(super) enum ConnState {
    Disconnected(Option<String>),
    Connecting,
    Connected,
}

#[derive(PartialEq, Clone, Copy)]
pub(super) enum LeftTab {
    Files,
    Search,
    Tags,
}

#[derive(PartialEq, Clone, Copy)]
pub(super) enum SettingsTab {
    Connection,
    Vault,
    Appearance,
    Plugins,
    Github,
    About,
}

pub(super) struct OpenNote {
    pub(super) id: Uuid,
    pub(super) path: String,
    pub(super) content: String,
    pub(super) version: i32,
    /// Content as last persisted in the database
    pub(super) saved: String,
    pub(super) last_edit: Option<f64>,
    /// Content currently being saved
    pub(super) saving: Option<String>,
    pub(super) conflict: Option<Conflict>,
    pub(super) remote_flash: f64,
}

#[derive(Default, Clone)]
pub(super) struct Tab {
    /// the dashboard tab (projects / tickets) instead of a note
    pub(super) dashboard: bool,
    /// la vue graphe des liens
    pub(super) graph: bool,
    pub(super) note: Option<Uuid>,
    pub(super) back: Vec<Uuid>,
    pub(super) fwd: Vec<Uuid>,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum OpenMode {
    /// open in the current tab, pushing the previous note to its history
    Replace,
    NewTab,
    /// navigation (back / forward / tab switch): no history change
    History,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum SortOrder {
    NameAsc,
    NameDesc,
    Modified,
}

#[derive(PartialEq, Clone, Copy)]
pub(super) enum ConflictTab {
    Resolve,
    Diff,
}

/// A divergence between the local editor and a newer version in the database,
/// resolved hunk by hunk (three-way merge, base = last known saved version).
pub(super) struct Conflict {
    pub(super) remote: Note,
    pub(super) blocks: Vec<Block>,
    pub(super) tab: ConflictTab,
    pub(super) diff_against_result: bool,
    pub(super) window_open: bool,
}

impl OpenNote {
    pub(super) fn dirty(&self) -> bool {
        self.content != self.saved || self.saving.is_some()
    }
}

pub(super) enum Dialog {
    NewNote { input: String, error: Option<String> },
    NewFolder { input: String, error: Option<String> },
    RenameNote { id: Uuid, input: String, error: Option<String> },
    RenameFolder { from: String, input: String, error: Option<String> },
    DeleteNote { id: Uuid, path: String },
    DeleteFolder { path: String },
    Switcher { query: String, sel: usize },
    Palette { query: String, sel: usize },
}

#[derive(Clone)]
pub(super) enum UiAction {
    OpenId(Uuid),
    OpenLink(String),
    OpenTag(String),
    NewNoteIn(String),
    NewFolderIn(String),
    RenameNote(Uuid, String),
    RenameFolder(String),
    DeleteNote(Uuid, String),
    DeleteFolder(String),
    Core(CoreCmd),
    Plugin(usize, String),
    OpenNewTab(Uuid),
    SwitchTab(usize),
    CloseTab(usize),
    CloseOtherTabs(usize),
    ToggleFolder(String),
    SetSort(SortOrder),
    Back,
    Forward,
    /// glisser-déposer dans l'explorateur : élément déplacé, dossier de destination ("" = racine)
    Move(DragItem, String),
    /// fichiers / dossiers déposés depuis le système : chemins, dossier de destination
    ImportFiles(Vec<std::path::PathBuf>, String),
}

/// Ce que l'on fait glisser dans l'explorateur.
#[derive(Clone, Debug)]
pub(super) enum DragItem {
    Note(Uuid, String),
    Folder(String),
}

impl DragItem {
    pub(super) fn path(&self) -> &str {
        match self {
            DragItem::Note(_, p) | DragItem::Folder(p) => p,
        }
    }
}

#[derive(Default)]
pub(super) struct TreeFolder {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) folders: BTreeMap<String, TreeFolder>,
    pub(super) notes: Vec<(String, Uuid, chrono::DateTime<chrono::Utc>)>,
}

pub(super) struct Autocomplete {
    /// char index where the partial link starts (just after `[[`)
    pub(super) start: usize,
    pub(super) partial: String,
    pub(super) sel: usize,
    pub(super) matches: Vec<String>,
}

/// Obsidian-style CSS themes / snippets currently loaded.
#[derive(Default)]
pub(super) struct CssState {
    pub(super) themes: Vec<(String, std::path::PathBuf)>,
    pub(super) snippets: Vec<(String, std::path::PathBuf)>,
    /// files read for the current override, with their modification time (hot reload)
    pub(super) watched: Vec<(std::path::PathBuf, Option<std::time::SystemTime>)>,
    pub(super) active_theme: Option<String>,
    pub(super) active_snippets: Vec<String>,
    pub(super) report: Vec<String>,
    pub(super) last_check: f64,
    pub(super) obsidian_input: String,
}

#[derive(Default)]
pub(super) struct Needs {
    pub(super) board: bool,
    pub(super) notes: bool,
    pub(super) folders: bool,
    pub(super) backlinks: bool,
    pub(super) graph: bool,
    pub(super) tags: bool,
    pub(super) plugins: bool,
}
