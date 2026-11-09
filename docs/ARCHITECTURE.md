# Architecture de Mnemo

Ce document explique comment le code est organisé et comment les morceaux communiquent.
Commencez par lire [Les trois threads](#les-trois-threads), le reste en découle.

## Les trois threads

```
        thread UI (egui)                 thread local (std)                 thread worker (tokio)
 ┌──────────────────────────┐  Cmd  ┌──────────────────────────┐     ┌─────────────────────────────┐
 │ MnemoApp::ui()           │ ────► │ local::Store             │     │ Worker::run                 │──► PostgreSQL
 │   handle_messages()      │ ◄──── │  notes .md, index,       │     │  tableau de bord, GitHub,   │◄── (requêtes)
 │   … dessin …             │  Msg  │  .mnemo/ (icônes…)       │     │  vault_sync (toutes les 2 s)│
 └──────────────────────────┘       │  relit le disque (1 s)   │     │ listen_loop (2ᵉ connexion)  │◄── NOTIFY
        │  Cmd (tableau de bord,    └────────────┬─────────────┘     └──────────────┬──────────────┘
        └──── connexion, GitHub) ──────────────────────────────────────────────────►│
                                                 └──── dossier .md + .mnemo/ids.json ┘ (partagés)
```

- `DbHandle::send` aiguille chaque `db::Cmd` : notes, dossiers, recherche, rétroliens, tags, graphe, icônes et plugins
  vont au **stockage local** (`src/local/`) ; connexion, tableau de bord et GitHub vont au **worker PostgreSQL**.
  Les deux renvoient des `db::Msg` sur le même canal, lus dans `handle_messages` sans jamais bloquer.
- Le dossier des notes est la référence : Mnemo fonctionne sans base. Le stockage local relit le disque toutes les
  secondes et transforme chaque changement extérieur en `Msg::LocalEvent`.
- Si une base est configurée, le worker la compare au dossier toutes les 2 s (`db/vault_sync.rs`), sans dépendre de
  l'affichage (fenêtre réduite comprise), se reconnecte seul toutes les 30 s, et `listen_loop` relaie les NOTIFY des
  autres appareils (`Msg::Event`) pour lancer la comparaison tout de suite.
- Stockage local et synchronisation partagent `local::shared` : l'identifiant de chaque note (`.mnemo/ids.json`, le
  même en local et en base) et un verrou pour les fichiers d'état `.mnemo/icons.json` / `plugins.json`.

## Carte des modules

### `src/` : le domaine et le worker

| Module | Rôle |
|---|---|
| `main.rs` | point d'entrée : options de la fenêtre, message d'aide si une bibliothèque graphique manque |
| `model.rs` | types partagés : `Note`, `NoteMeta`, `VaultEvent`, `GraphData`… |
| `config.rs` | configuration locale (URL de la base, préférences), lue et écrite en TOML |
| `schema.sql` | tables, triggers (version, liens, `pg_notify`). Rejoué à chaque connexion : il doit rester **idempotent** |
| `db/mod.rs` | `Cmd`, `Msg`, `DbHandle`, `spawn`, `Worker::run` et l'aiguillage `handle` |
| `db/connection.rs` | ouverture (TLS si `sslmode=require`), migration sous verrou consultatif, boucle `LISTEN` avec backoff |
| `db/notes.rs` | commandes notes / dossiers / recherche / graphe / plugins, réécriture des `[[liens]]` au renommage |
| `db/board.rs` | commandes projets / tickets / rapports, chargement du tableau de bord |
| `db/vault_sync.rs` | synchro dossier ↔ base : notes, dossiers, renommages, suppressions, icônes et plugins |
| `local/mod.rs` | stockage local : commandes des notes sur les fichiers, relecture du disque |
| `local/index.rs` | liens, résolution, tags, recherche, rétroliens, graphe, réécriture des liens (fonctions pures) |
| `local/shared.rs` | identifiants des notes (`.mnemo/ids.json`) et fichiers d'état partagés avec la synchro |
| `db/github_sync.rs` | publication et synchronisation des tickets avec les issues GitHub |
| `board.rs` | domaine du tableau de bord et **plan** de synchronisation GitHub (fonctions pures, testées) |
| `github.rs` | client REST GitHub bloquant (appelé via `spawn_blocking`) |
| `merge.rs` | fusion à 3 voies et diff unifié (testés) |
| `markdown.rs` | transforme `[[wikilinks]]` et `#tags` en liens `mnemo:` cliquables |
| `plugins.rs` | hôte de plugins Rhai (voir [plugins.md](plugins.md)) |
| `css.rs` | lit les thèmes / snippets CSS au format Obsidian (voir [themes.md](themes.md)) |
| `theme.rs` | palette, polices, visuels egui, coloration de l'éditeur, widgets maison |
| `graph.rs` | vue graphe (disposition par forces) |

### `src/app/` : l'interface

`app.rs` contient la structure `MnemoApp` (tout l'état de l'interface), sa construction et la boucle `ui()`.
Chaque sous-module ajoute des méthodes à `MnemoApp` dans son propre `impl MnemoApp { … }` et commence par
`use super::*;`.

| Module | Rôle |
|---|---|
| `state.rs` | petits types d'état : onglets, dialogues, conflits, `UiAction`, `Needs`… |
| `sync.rs` | connexion, traitement des `Msg`, événements live, rechargements groupés (`flush_needs`) |
| `notes.rs` | ouverture / onglets / historique de navigation, sauvegarde automatique |
| `layout.rs` | écran de connexion, barre d'onglets, en-tête de vue, indicateur de synchronisation, notifications |
| `sidebar.rs` | ruban, explorateur (arbre de dossiers, glisser-déposer), panneau de droite |
| `editor.rs` | éditeur, titre intégré, autocomplétion `[[`, aperçu |
| `commands.rs` | `CoreCmd` (palette Ctrl+P), raccourcis, `UiAction`, actions des plugins |
| `dialogs.rs` | palette, sélecteur de notes, saisie de nom, confirmations |
| `conflict.rs` | fenêtre de résolution de conflit (choix par bloc, diff) |
| `settings.rs` | fenêtre des paramètres (général, plugins, GitHub) |
| `icons.rs` | icônes des dossiers / notes : fenêtre de choix (Lucide, emoji), import depuis Iconize |
| `vault.rs` | onglet Coffre Obsidian, passage de synchronisation périodique, import des fichiers déposés, `settings_row` |
| `appearance.rs` | onglet Apparence, chargement et surveillance des fichiers CSS |
| `dashboard/` | tableau de bord : `mod.rs` (état, vue principale), `rows.rs`, `columns.rs`, `ticket.rs`, `actions.rs` |

## Une frame de `MnemoApp::ui()`

Dans l'ordre :

1. `watch_css`, `apply_theme` : recharge le CSS s'il a changé sur le disque.
2. `handle_messages` : applique tout ce que le worker a renvoyé.
3. `apply_plugin_actions`, `tick_autosave` : actions demandées par les plugins, sauvegarde après 0,6 s d'inactivité.
4. `flush_needs` : envoie **une seule fois** les rechargements demandés pendant la frame
   (plusieurs événements live → une seule requête `ListNotes`).
5. Dessin : ruban, barres latérales, onglets, éditeur ou tableau de bord, fenêtres et dialogues.
6. `process_actions` : les clics ne modifient pas l'état pendant le dessin. Ils empilent des `UiAction`,
   traitées en fin de frame (même principe pour `DashAction` dans le tableau de bord).

## Flux de synchronisation d'une note

1. **Frappe** : `editor` marque l'onglet modifié. `tick_autosave` envoie `Cmd::Save { id, content, base_version }`
   au stockage local, qui écrit le fichier (écriture atomique) et renvoie `Msg::Saved`. Si la note a changé sur le
   disque entre-temps (autre éditeur, synchro), sa version a augmenté : `Msg::Conflict`, puis fusion à 3 voies.
2. **Base, si configurée** : au passage suivant (≤ 2 s), `vault_sync` voit le fichier modifié et met la note à jour
   dans une transaction marquée `<client_id>#disk`. Les triggers incrémentent `version` et font `pg_notify`.
3. **Autres appareils** : `listen_loop` reçoit le NOTIFY (`Msg::Event`) ; l'interface demande aussitôt un passage
   (`Cmd::VaultScan`) qui écrit le fichier. Leur stockage local le voit (≤ 1 s) et envoie `Msg::LocalEvent` :
   la note ouverte est rechargée (`apply_refresh`), fusionnée si elle était en cours de modification.
4. **Perte de connexion** : le worker passe hors ligne (`Msg::ConnectionFailed`, une seule alerte), réessaie toutes
   les 30 s et, au retour, recompare tout le dossier avec la base.

## Recettes

### Ajouter une commande à la palette (Ctrl+P)

Dans `src/app/commands.rs` :

1. ajouter une variante à `enum CoreCmd` ;
2. l'ajouter à `CORE_COMMANDS` avec son libellé et son raccourci affiché (`""` si aucun) ;
3. traiter la variante dans `run_core` ;
4. si elle a un raccourci, l'ajouter dans `shortcuts`.

### Ajouter une commande au worker

1. `src/db/mod.rs` : ajouter la variante à `Cmd` (et, si elle renvoie un résultat, à `Msg`).
2. Implémenter la branche dans `note_cmd` (`db/notes.rs`), `board_cmd` (`db/board.rs`) ou `github_cmd`
   (`db/github_sync.rs`). Pour une commande projets / tickets / GitHub, l'ajouter aussi à l'aiguillage
   de `Worker::handle`. Les autres commandes vont à `note_cmd` par défaut.
3. Les écritures passent par `tagged_tx` pour que les autres instances sachent d'où vient la modification.
4. `src/app/sync.rs` : traiter le nouveau `Msg` dans `handle_messages`.

### Modifier le schéma

`schema.sql` est rejoué à **chaque** connexion (sous `pg_advisory_lock`). N'utilisez que des instructions
idempotentes : `CREATE TABLE IF NOT EXISTS`, `ALTER TABLE … ADD COLUMN IF NOT EXISTS`, `CREATE OR REPLACE FUNCTION`…

## Le patch `vendor/egui_commonmark_backend`

Le mode lecture utilise `egui_commonmark`. Par défaut, son backend affiche le gras en changeant seulement la
couleur. Mnemo utilise une copie locale de `egui_commonmark_backend` **0.26.0**, branchée dans `Cargo.toml` par :

```toml
[patch.crates-io]
egui_commonmark_backend = { path = "vendor/egui_commonmark_backend" }
```

La seule modification se trouve dans `src/misc.rs`, au commentaire `// Mnemo patch` : le gras et les titres
de niveau 1 à 3 utilisent la vraie police Inter SemiBold enregistrée par l'application.

**Quand vous mettez à jour `egui_commonmark`** :

1. récupérez la nouvelle version du backend (`cargo download egui_commonmark_backend==X.Y.Z`, ou copiez-la
   depuis `~/.cargo/registry/src/*/egui_commonmark_backend-X.Y.Z`) ;
2. remplacez `vendor/egui_commonmark_backend`, puis réappliquez le bloc `// Mnemo patch` dans `src/misc.rs` ;
3. vérifiez que les versions du backend et de `egui_commonmark` correspondent. Sinon, Cargo ignore le patch
   et affiche l'avertissement « patch was not used ».

## Vérifier avant de commiter

```bash
make check   # cargo fmt --check + clippy -D warnings + tests
```

La CI (`.github/workflows/release.yml`) lance la même commande avant de construire les binaires.
