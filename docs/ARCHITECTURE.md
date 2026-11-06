# Architecture de Mnemo

Ce document explique comment le code est organisé et comment les morceaux communiquent.
Commencez par lire [Les deux threads](#les-deux-threads), le reste en découle.

## Les deux threads

```
          thread UI (egui)                           thread worker (tokio)
 ┌────────────────────────────────┐   Cmd    ┌──────────────────────────────┐
 │ MnemoApp::ui() à chaque frame  │ ───────► │ Worker::run → handle(cmd)    │──► PostgreSQL
 │   handle_messages()            │ ◄─────── │   note_cmd / board_cmd /     │◄── (requêtes)
 │   … dessin des panneaux …      │   Msg    │   github_cmd                 │
 └────────────────────────────────┘          │ listen_loop (2ᵉ connexion)   │◄── NOTIFY vault_events
                                             └──────────────────────────────┘
```

- L'interface ne parle **jamais** directement à Postgres. Elle envoie des `db::Cmd` via `DbHandle::send`
  (canal tokio) et lit les `db::Msg` dans `handle_messages` (canal std), sans jamais bloquer.
- Le worker exécute les commandes **une par une**, sur une seule connexion de requêtes.
  Une seconde connexion fait `LISTEN vault_events` et relaie chaque événement en `Msg::Event`.
- Après chaque message, le worker appelle `ctx.request_repaint()` pour réveiller l'interface.

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
| `layout.rs` | écran de connexion, barre d'onglets, en-tête de vue, barre d'état, notifications |
| `sidebar.rs` | ruban, explorateur (arbre de dossiers), panneau de droite |
| `editor.rs` | éditeur, titre intégré, autocomplétion `[[`, aperçu |
| `commands.rs` | `CoreCmd` (palette Ctrl+P), raccourcis, `UiAction`, actions des plugins |
| `dialogs.rs` | palette, sélecteur de notes, saisie de nom, confirmations |
| `conflict.rs` | fenêtre de résolution de conflit (choix par bloc, diff) |
| `settings.rs` | fenêtre des paramètres (général, plugins, GitHub) |
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

1. **Frappe** : `editor` marque l'onglet modifié. `tick_autosave` envoie `Cmd::Save { id, content, base_version }`.
2. **Worker** : `UPDATE … WHERE version = base_version`, dans une transaction marquée avec le `client_id`
   (`tagged_tx`). Les triggers incrémentent `version`, remplissent `updated_by` et font `pg_notify`.
   - Version à jour → `Msg::Saved(note)`.
   - Version dépassée (quelqu'un a écrit entre-temps) → `Msg::Conflict(note_actuelle)`.
3. **Autres instances** : `listen_loop` reçoit le NOTIFY, puis `Msg::Event` arrive dans `handle_event`.
   L'instance ignore ses propres échos (`updated_by == client_id`). Sinon, elle recharge la note
   (`LoadPurpose::Refresh`), puis `apply_refresh` :
   - pas de modification locale → la note est remplacée ;
   - modifications locales → `on_divergence` : fusion à 3 voies (`merge.rs`) avec comme base la dernière
     version enregistrée connue. Les blocs qui se chevauchent ouvrent la fenêtre de conflit.
4. **Perte de connexion** : le worker rouvre la connexion avant la commande suivante, et `listen_loop`
   se reconnecte avec un backoff (0,5 s → 30 s) puis envoie `Msg::Resync` pour tout recharger.

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
