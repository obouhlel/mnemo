# Écrire un plugin

Un plugin est un dossier dans le dossier des plugins (visible dans ⚙ Paramètres › Plugins) :

```
mon-plugin/
├── plugin.toml
└── main.rhai
```

```toml
id = "mon-plugin"
name = "Mon plugin"
version = "1.0.0"
description = "Ce que fait le plugin"

[[settings]]          # optionnel : réglages éditables dans l'interface
key = "prefixe"
label = "Préfixe"
default = "TODO"      # texte, nombre ou booléen ; kind = "multiline" pour un champ multiligne
```

Tous les hooks sont optionnels (`note` est une map `#{ id, path, title, content }`, ou `()` si aucune note n'est ouverte) :

```rhai
fn on_load() {}                              // à l'activation
fn commands() { [#{ id: "go", name: "Ma commande" }] }   // apparaissent dans Ctrl+P
fn run(command_id, note) { insert_text("Bonjour"); }
fn status(note) { "texte de la barre d'état" }
fn panel(note) { "## Markdown affiché à droite" }
fn preprocess(markdown, note) { markdown }   // transforme l'aperçu (pas la note)
fn on_open(note) {}
fn on_save(note) {}
fn on_event(event) {}                        // #{ kind, op, path } : événements live de la base
```

API disponible : `notice(msg)`, `log(msg)`, `open_note(path)`, `create_note(path, contenu)` (ouvre si elle existe),
`insert_text(t)`, `replace_content(t)`, `setting(clé)`, `note_paths()`, `date(fmt)`, `date_offset(jours, fmt)`,
`word_count(t)`, `char_count(t)`, `state_get(clé)`, `state_set(clé, valeur)`.

Plugins fournis (copiés au premier lancement, désactivés par défaut) : **Compteur de mots**, **Notes quotidiennes**,
**Plan** (table des matières), **Encadrés** (`> [!note]`, `> [!warning]`…), **Modèles**.
Les scripts sont limités en nombre d'opérations : un plugin bogué ne peut pas figer l'application.
