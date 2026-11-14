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

## Bac à sable

Un script n'a accès qu'au langage Rhai et à l'API ci-dessus : ni fichiers, ni réseau, ni processus. Ce qui est imposé :

- **`main`** (facultatif dans `plugin.toml`, `main.rhai` par défaut) est un simple nom de fichier du dossier du
  plugin : un chemin absolu, `..` ou un sous-dossier sont refusés ;
- **pas d'`import`** : tout le script tient dans ce fichier ;
- **temps** : `status`, `panel`, `preprocess` et `commands` (rappelés à chaque modification de la note) ont droit à
  1 000 000 d'opérations et 2 s par appel ; les autres hooks (`run`, `on_load`, `on_open`, `on_save`, `on_event`) à
  5 000 000 d'opérations et 10 s ;
- **mémoire** : chaînes de 10 Mo, tableaux de 100 000 éléments et maps de 10 000 entrées au plus (éléments imbriqués
  compris), 64 niveaux d'appels, 1 000 clés `state_set` par plugin, 200 actions (`notice`, `insert_text`…) par appel ;
- **erreurs** : une limite dépassée, un format de date invalide (`date("%Q")`), un décalage de date hors calendrier ou
  une erreur interne arrêtent l'appel ; le plugin est mis en erreur (message affiché, bouton pour le relancer dans les
  paramètres) et l'application continue.

Les hooks s'exécutent dans le thread de l'interface : un script lent la ralentit, dans les limites ci-dessus.
